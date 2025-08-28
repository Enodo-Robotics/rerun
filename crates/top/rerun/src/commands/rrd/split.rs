use std::io::Write as _;
use std::path::Path;

use anyhow::Context as _;

use re_chunk_store::ChunkStoreConfig;
use re_entity_db::EntityDb;
use re_log_types::{LogMsg, StoreId};
use re_sdk::StoreKind;

use crate::commands::read_rrd_streams_from_file_or_stdin;

// Helper function to determine if a message is static (timeline-less)
fn is_static_message(log_msg: &LogMsg) -> bool {
    match log_msg {
        LogMsg::ArrowMsg(_, arrow_msg) => {
            // Check if the timepoint_max is empty (static)
            arrow_msg.timepoint_max.is_static()
        }
        LogMsg::SetStoreInfo(_) => {
            // Store info is typically static metadata
            true
        }
        LogMsg::BlueprintActivationCommand(_) => {
            // Blueprint commands are typically static
            true
        }
    }
}

// ---

#[derive(Debug, Clone, clap::Parser)]
pub struct SplitCommand {
    /// Paths to read from. Reads from standard input if none are specified.
    path_to_input_rrds: Vec<String>,

    /// Output directory for split files. Required.
    #[arg(short = 'o', long = "output-dir", value_name = "dir")]
    output_dir: String,

    /// Base name for output files. Defaults to "chunk".
    #[arg(short = 'n', long = "name", default_value = "chunk")]
    base_name: String,

    /// Maximum size per output file in bytes. Defaults to 100MB.
    #[arg(short = 's', long = "size", default_value = "104857600")]
    max_size_bytes: u64,

    /// Exclude static (timeline-less) data from the output. By default, static data is included.
    #[arg(long = "exclude-static")]
    exclude_static: bool,

    /// If set, will try to proceed even in the face of IO and/or decoding errors in the input data.
    #[clap(long = "continue-on-error", default_value_t = false)]
    continue_on_error: bool,
}

impl SplitCommand {
    pub fn run(&self) -> anyhow::Result<()> {
        let Self {
            path_to_input_rrds,
            output_dir,
            base_name,
            max_size_bytes,
            exclude_static,
            continue_on_error,
        } = self;

        // Validate output directory
        let output_path = Path::new(output_dir);
        if !output_path.exists() {
            std::fs::create_dir_all(output_path)
                .with_context(|| format!("couldn't create output directory {output_dir:?}"))?;
        }

        anyhow::ensure!(
            output_path.is_dir(),
            "output path {output_dir:?} is not a directory"
        );

        anyhow::ensure!(
            *max_size_bytes > 1024,
            "max size must be at least 1KB, got {}",
            max_size_bytes
        );


        // NOTE: We're doing headless processing, there's no point in running subscribers, it will just
        // (massively) slow us down.
        let store_config = ChunkStoreConfig::ALL_DISABLED;

        split_recording(
            *continue_on_error,
            &store_config,
            path_to_input_rrds,
            output_dir,
            base_name,
            *max_size_bytes,
            *exclude_static,
        )
    }
}

fn split_recording(
    continue_on_error: bool,
    store_config: &ChunkStoreConfig,
    path_to_input_rrds: &[String],
    output_dir: &str,
    base_name: &str,
    max_size_bytes: u64,
    exclude_static: bool,
) -> anyhow::Result<()> {
    let file_size_to_string = |size: Option<u64>| {
        size.map_or_else(
            || "<unknown>".to_owned(),
            |size| re_format::format_bytes(size as _),
        )
    };

    let now = std::time::Instant::now();
    re_log::info!(
        srcs = ?path_to_input_rrds,
        output_dir,
        base_name,
        max_size_bytes = %file_size_to_string(Some(max_size_bytes)),
        "split started"
    );

    let (rx, rx_size_bytes) = read_rrd_streams_from_file_or_stdin(path_to_input_rrds);

    // Collect all messages first to ensure proper ordering
    let mut all_messages = Vec::new();
    let mut store_info_messages = Vec::new(); // Collect StoreInfo messages separately
    let mut entity_dbs: std::collections::HashMap<StoreId, EntityDb> = Default::default();

    for (_source, res) in rx {
        let mut is_success = true;

        match res {
            Ok(msg) => {
                // Collect StoreInfo messages separately to include in each chunk
                if matches!(msg, re_log_types::LogMsg::SetStoreInfo(_)) {
                    store_info_messages.push(msg.clone());
                }
                
                // Store the message for later processing
                all_messages.push(msg.clone());
                
                // Also add to entity_db for metadata extraction
                if let Err(err) = entity_dbs
                    .entry(msg.store_id().clone())
                    .or_insert_with(|| {
                        re_entity_db::EntityDb::with_store_config(
                            msg.store_id().clone(),
                            store_config.clone(),
                        )
                    })
                    .add(&msg)
                {
                    re_log::error!(%err, "couldn't index corrupt chunk");
                    is_success = false;
                }
            }

            Err(err) => {
                re_log::error!(err = re_error::format(err));
                is_success = false;
            }
        }

        if !continue_on_error && !is_success {
            anyhow::bail!(
                "one or more IO and/or decoding failures in the input stream (check logs)"
            )
        }
    }

    // Sort messages to ensure consistent output
    // Blueprint messages first, then recording messages
    all_messages.sort_by_key(|msg| {
        let store_kind_priority = match entity_dbs.get(msg.store_id()) {
            Some(db) => match db.store_kind() {
                StoreKind::Blueprint => 0,
                StoreKind::Recording => 1,
            },
            None => 2,
        };
        
        (store_kind_priority, msg.store_id().clone())
    });

    // Get version and encoding options from the first entity db
    let version = entity_dbs
        .values()
        .next()
        .and_then(|db| db.store_info())
        .and_then(|info| info.store_version)
        .unwrap_or(re_build_info::CrateVersion::LOCAL);
    let encoding_options = re_log_encoding::EncodingOptions::PROTOBUF_COMPRESSED;

    // Split messages into chunks based on size
    let mut chunk_index = 0;
    let mut current_chunk = Vec::new();
    let mut current_size = 0u64;
    let mut total_output_size = 0u64;
    let mut files_created = 0;

    // Estimate message size for splitting decisions
    let estimate_message_size = |msg: &LogMsg| -> u64 {
        match msg {
            LogMsg::ArrowMsg(_, arrow_msg) => {
                // Rough estimate based on arrow data
                arrow_msg.batch.get_array_memory_size() as u64 + 1024 // Add overhead
            }
            LogMsg::SetStoreInfo(_) => 1024, // Small fixed size
            LogMsg::BlueprintActivationCommand(_) => 512, // Small fixed size
        }
    };

    for msg in all_messages {
        let is_static = is_static_message(&msg);
        
        // Apply static filtering
        if exclude_static && is_static {
            continue;
        }
        
        let msg_size = estimate_message_size(&msg);
        
        // If adding this message would exceed the limit and we have messages in the current chunk
        if current_size + msg_size > max_size_bytes && !current_chunk.is_empty() {
            // Write current chunk (including StoreInfo at the beginning)
            let chunk_path = format!("{}/{}_part_{:03}.rrd", output_dir, base_name, chunk_index);
            let chunk_with_store_info = prepend_store_info(&store_info_messages, &current_chunk);
            let chunk_size = write_chunk(&chunk_with_store_info, &chunk_path, version, encoding_options)?;
            
            re_log::info!(
                chunk = chunk_index,
                path = %chunk_path,
                size = %file_size_to_string(Some(chunk_size)),
                messages = current_chunk.len(),
                "wrote chunk"
            );
            
            total_output_size += chunk_size;
            files_created += 1;
            chunk_index += 1;
            
            // Start new chunk
            current_chunk.clear();
            current_size = 0;
        }
        
        current_chunk.push(msg);
        current_size += msg_size;
    }

    // Write the final chunk if it has any messages
    if !current_chunk.is_empty() {
        let chunk_path = format!("{}/{}_part_{:03}.rrd", output_dir, base_name, chunk_index);
        let chunk_with_store_info = prepend_store_info(&store_info_messages, &current_chunk);
        let chunk_size = write_chunk(&chunk_with_store_info, &chunk_path, version, encoding_options)?;
        
        re_log::info!(
            chunk = chunk_index,
            path = %chunk_path,
            size = %file_size_to_string(Some(chunk_size)),
            messages = current_chunk.len(),
            "wrote chunk"
        );
        
        total_output_size += chunk_size;
        files_created += 1;
    }

    // Generate merge script for convenience
    let merge_script_path = format!("{}/merge_chunks.sh", output_dir);
    let merge_command = format!(
        "#!/bin/bash\n# Generated merge script to reconstruct original file\nrerun rrd merge {}/{}_part_*.rrd -o {}/{}_merged.rrd\n",
        output_dir, base_name, output_dir, base_name
    );
    std::fs::write(&merge_script_path, merge_command)
        .with_context(|| format!("couldn't write merge script to {merge_script_path:?}"))?;
    
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&merge_script_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&merge_script_path, perms)?;
    }

    let rrds_in_size = rx_size_bytes.recv().ok();

    re_log::info!(
        total_output_size = %file_size_to_string(Some(total_output_size)),
        time = ?now.elapsed(),
        files_created,
        srcs = ?path_to_input_rrds,
        srcs_size_bytes = %file_size_to_string(rrds_in_size),
        merge_script = %merge_script_path,
        "split finished"
    );

    Ok(())
}

fn write_chunk(
    messages: &[LogMsg],
    output_path: &str,
    version: re_build_info::CrateVersion,
    encoding_options: re_log_encoding::EncodingOptions,
) -> anyhow::Result<u64> {
    let mut rrd_out = std::io::BufWriter::new(
        std::fs::File::create(output_path).with_context(|| format!("{output_path:?}"))?,
    );

    let size = re_log_encoding::encoder::encode(
        version,
        encoding_options,
        messages.iter().cloned().map(Ok),
        &mut rrd_out,
    )
    .context("couldn't encode messages")?;

    rrd_out.flush().context("couldn't flush output")?;

    Ok(size)
}

/// Prepends StoreInfo messages to the beginning of a chunk to preserve original application_id
fn prepend_store_info(
    store_info_messages: &[re_log_types::LogMsg],
    chunk_messages: &[re_log_types::LogMsg],
) -> Vec<re_log_types::LogMsg> {
    let mut result = Vec::with_capacity(store_info_messages.len() + chunk_messages.len());
    
    // Add all StoreInfo messages first
    result.extend_from_slice(store_info_messages);
    
    // Add the chunk messages, but skip any StoreInfo messages to avoid duplicates
    for msg in chunk_messages {
        if !matches!(msg, re_log_types::LogMsg::SetStoreInfo(_)) {
            result.push(msg.clone());
        }
    }
    
    result
}