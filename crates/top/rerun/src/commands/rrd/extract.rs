use std::io::{IsTerminal as _, Write as _};
use std::collections::HashMap;

use anyhow::Context as _;
use itertools::Either;

use re_chunk_store::ChunkStoreConfig;
use re_entity_db::EntityDb;
use re_log_types::{LogMsg, StoreId};
use re_chunk::Chunk;
use re_sdk::StoreKind;

use crate::commands::read_rrd_streams_from_file_or_stdin;

// ---

#[derive(Debug, Clone, clap::Parser)]
pub struct ExtractCommand {
    /// Paths to read from. Reads from standard input if none are specified.
    path_to_input_rrds: Vec<String>,

    /// Path to write to. Writes to standard output if unspecified.
    #[arg(short = 'o', long = "output", value_name = "dst.(rrd|rbl)")]
    path_to_output_rrd: Option<String>,

    /// Extract only static (timeline-less) data.
    #[arg(long = "static-only")]
    static_only: bool,

    /// Extract only temporal (with timeline) data.
    #[arg(long = "temporal-only")]
    temporal_only: bool,

    /// List available entity paths instead of extracting data.
    #[arg(long = "list-entities")]
    list_entities: bool,

    /// Extract data for specific entity path(s) only. Can be specified multiple times.
    /// Use glob patterns like '/world/**' to match multiple paths.
    #[arg(short = 'e', long = "entity-path")]
    entity_paths: Vec<String>,

    /// Output format: 'rrd' (default), 'json', or 'csv'.
    #[arg(long = "format", default_value = "rrd")]
    output_format: String,

    /// If set, will try to proceed even in the face of IO and/or decoding errors in the input data.
    #[clap(long = "continue-on-error", default_value_t = false)]
    continue_on_error: bool,
}

impl ExtractCommand {
    pub fn run(&self) -> anyhow::Result<()> {
        let Self {
            path_to_input_rrds,
            path_to_output_rrd,
            static_only,
            temporal_only,
            list_entities,
            entity_paths,
            output_format,
            continue_on_error,
        } = self;

        // Validate conflicting options
        if *static_only && *temporal_only {
            anyhow::bail!("Cannot specify both --static-only and --temporal-only");
        }

        if (*static_only || *temporal_only) && !entity_paths.is_empty() {
            anyhow::bail!("Cannot specify --entity-path with --static-only or --temporal-only");
        }

        // Validate output format
        match output_format.as_str() {
            "rrd" | "json" | "csv" => {}
            _ => anyhow::bail!("Invalid output format '{}'. Supported formats: rrd, json, csv", output_format),
        }

        if path_to_output_rrd.is_none() && output_format == "rrd" && !*list_entities {
            anyhow::ensure!(
                !std::io::stdout().is_terminal(),
                "you must redirect the output to a file and/or stream when using RRD format"
            );
        }

        // Parse entity path patterns
        let entity_path_filter: Option<Vec<String>> = if entity_paths.is_empty() {
            None
        } else {
            Some(entity_paths.clone())
        };

        // NOTE: We're doing headless processing, there's no point in running subscribers, it will just
        // (massively) slow us down.
        let store_config = ChunkStoreConfig::ALL_DISABLED;

        extract_recording(
            *continue_on_error,
            &store_config,
            path_to_input_rrds,
            path_to_output_rrd.as_ref(),
            *static_only,
            *temporal_only,
            *list_entities,
            entity_path_filter,
            output_format,
        )
    }
}

fn extract_recording(
    continue_on_error: bool,
    store_config: &ChunkStoreConfig,
    path_to_input_rrds: &[String],
    path_to_output_rrd: Option<&String>,
    static_only: bool,
    temporal_only: bool,
    list_entities: bool,
    entity_path_filter: Option<Vec<String>>,
    output_format: &str,
) -> anyhow::Result<()> {
    let file_size_to_string = |size: Option<u64>| {
        size.map_or_else(
            || "<unknown>".to_owned(),
            |size| re_format::format_bytes(size as _),
        )
    };

    let now = std::time::Instant::now();
    
    let extraction_info = if static_only {
        "static data only".to_owned()
    } else if temporal_only {
        "temporal data only".to_owned()
    } else if let Some(ref paths) = entity_path_filter {
        format!("entity paths: {}", paths.join(", "))
    } else {
        "all data".to_owned()
    };

    re_log::info!(
        srcs = ?path_to_input_rrds,
        extraction = %extraction_info,
        format = %output_format,
        "extract started"
    );

    let (rx, rx_size_bytes) = read_rrd_streams_from_file_or_stdin(path_to_input_rrds);

    let mut entity_dbs: HashMap<StoreId, EntityDb> = Default::default();
    let mut store_info_messages: Vec<LogMsg> = Vec::new();

    for (_source, res) in rx {
        let mut is_success = true;

        match res {
            Ok(msg) => {
                // Collect StoreInfo messages to preserve application_id
                if matches!(msg, LogMsg::SetStoreInfo(_)) {
                    store_info_messages.push(msg.clone());
                }
                
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

    // If listing entities, just print them and return
    if list_entities {
        list_available_entities(&entity_dbs)?;
        return Ok(());
    }

    // Filter and extract data based on options
    let extracted_messages = extract_filtered_messages(
        &entity_dbs,
        static_only,
        temporal_only,
        entity_path_filter,
    )?;

    // Output the extracted data
    match output_format {
        "rrd" => write_rrd_output(store_info_messages.clone(), extracted_messages, path_to_output_rrd, &entity_dbs)?,
        "json" => write_json_output(store_info_messages.clone(), extracted_messages, path_to_output_rrd)?,
        "csv" => write_csv_output(store_info_messages.clone(), extracted_messages, path_to_output_rrd)?,
        _ => unreachable!("Invalid output format should have been caught earlier"),
    }

    let rrds_in_size = rx_size_bytes.recv().ok();

    re_log::info!(
        time = ?now.elapsed(),
        srcs = ?path_to_input_rrds,
        srcs_size_bytes = %file_size_to_string(rrds_in_size),
        extraction = %extraction_info,
        format = %output_format,
        "extract finished"
    );

    Ok(())
}

fn list_available_entities(entity_dbs: &HashMap<StoreId, EntityDb>) -> anyhow::Result<()> {
    let mut all_entities = std::collections::BTreeSet::new();
    
    for entity_db in entity_dbs.values() {
        // Get all entity paths from the entity database
        for entity_path in entity_db.entity_paths() {
            all_entities.insert(entity_path.to_string());
        }
    }

    println!("Available entity paths:");
    if all_entities.is_empty() {
        println!("  (no entity paths found)");
    } else {
        for entity_path in &all_entities {
            println!("  - {}", entity_path);
        }
    }

    Ok(())
}

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

fn message_has_entity_path(log_msg: &LogMsg, entity_path_pattern: &str) -> bool {
    match log_msg {
        LogMsg::ArrowMsg(_, arrow_msg) => {
            // Convert ArrowMsg to Chunk to access entity path
            if let Ok(chunk) = Chunk::from_arrow_msg(arrow_msg) {
                let entity_path_str = chunk.entity_path().to_string();
                
                // Simple glob-like pattern matching
                if entity_path_pattern.ends_with("/**") {
                    // Match prefix
                    let prefix = &entity_path_pattern[..entity_path_pattern.len() - 3];
                    entity_path_str.starts_with(prefix)
                } else if entity_path_pattern.contains('*') {
                    // Basic wildcard matching
                    let pattern_parts: Vec<&str> = entity_path_pattern.split('*').collect();
                    if pattern_parts.len() == 2 {
                        entity_path_str.starts_with(pattern_parts[0]) && 
                        entity_path_str.ends_with(pattern_parts[1])
                    } else {
                        // Exact match for complex patterns
                        entity_path_str == entity_path_pattern
                    }
                } else {
                    // Exact match
                    entity_path_str == entity_path_pattern
                }
            } else {
                false
            }
        }
        _ => false, // Non-arrow messages don't have entity paths
    }
}

fn extract_filtered_messages(
    entity_dbs: &HashMap<StoreId, EntityDb>,
    static_only: bool,
    temporal_only: bool,
    entity_path_filter: Option<Vec<String>>,
) -> anyhow::Result<Vec<LogMsg>> {
    let mut messages = Vec::new();

    for entity_db in entity_dbs.values() {
        // Get all messages from the entity database
        let db_messages = if entity_db.store_kind() == StoreKind::Blueprint {
            // Blueprints are always included (unless filtered by static/temporal options)
            entity_db.to_messages(None).collect::<Vec<_>>()
        } else {
            // Recording data
            entity_db.to_messages(None).collect::<Vec<_>>()
        };

        for msg_result in db_messages {
            let msg = match msg_result {
                Ok(msg) => msg,
                Err(err) => {
                    re_log::warn!("Failed to decode message: {}", err);
                    continue;
                }
            };
            
            let is_static = is_static_message(&msg);
            
            // Apply static/temporal filtering
            if static_only && !is_static {
                continue;
            }
            if temporal_only && is_static {
                continue;
            }

            // Apply entity path filtering
            if let Some(ref entity_patterns) = entity_path_filter {
                let has_matching_path = entity_patterns.iter()
                    .any(|pattern| message_has_entity_path(&msg, pattern));
                
                if !has_matching_path {
                    continue;
                }
            }

            messages.push(msg);
        }
    }

    Ok(messages)
}

fn write_rrd_output(
    store_info_messages: Vec<LogMsg>,
    messages: Vec<LogMsg>,
    path_to_output_rrd: Option<&String>,
    entity_dbs: &HashMap<StoreId, EntityDb>,
) -> anyhow::Result<()> {
    let mut rrd_out = if let Some(path) = path_to_output_rrd {
        Either::Left(std::io::BufWriter::new(
            std::fs::File::create(path).with_context(|| format!("{path:?}"))?,
        ))
    } else {
        Either::Right(std::io::BufWriter::new(std::io::stdout().lock()))
    };

    // TODO(cmc): encoding options should match the original.
    let encoding_options = re_log_encoding::EncodingOptions::PROTOBUF_COMPRESSED;
    let version = entity_dbs
        .values()
        .next()
        .and_then(|db| db.store_info())
        .and_then(|info| info.store_version)
        .unwrap_or(re_build_info::CrateVersion::LOCAL);

    // Combine StoreInfo messages first, then the extracted messages
    let all_messages = store_info_messages.into_iter().chain(messages.into_iter());
    
    let _rrd_out_size = re_log_encoding::encoder::encode(
        version,
        encoding_options,
        all_messages.map(Ok),
        &mut rrd_out,
    )
    .context("couldn't encode messages")?;

    rrd_out.flush().context("couldn't flush output")?;

    Ok(())
}

fn write_json_output(
    store_info_messages: Vec<LogMsg>,
    messages: Vec<LogMsg>,
    path_to_output_rrd: Option<&String>,
) -> anyhow::Result<()> {
    let mut output: Box<dyn std::io::Write> = if let Some(path) = path_to_output_rrd {
        Box::new(std::io::BufWriter::new(
            std::fs::File::create(path).with_context(|| format!("{path:?}"))?,
        ))
    } else {
        Box::new(std::io::BufWriter::new(std::io::stdout().lock()))
    };

    writeln!(output, "[")?;
    
    // Combine StoreInfo messages first, then the extracted messages
    let all_messages: Vec<_> = store_info_messages.into_iter().chain(messages.into_iter()).collect();
    
    for (i, msg) in all_messages.iter().enumerate() {
        if i > 0 {
            writeln!(output, ",")?;
        }
        
        // Create a simplified JSON representation without external dependencies
        let json_str = match msg {
            LogMsg::ArrowMsg(_id, arrow_msg) => {
                format!(
                    r#"  {{
    "type": "ArrowMsg",
    "chunk_id": "{}",
    "timepoint": "{}",
    "num_rows": {},
    "is_static": {}
  }}"#,
                    arrow_msg.chunk_id,
                    format!("{:?}", arrow_msg.timepoint_max),
                    arrow_msg.batch.num_rows(),
                    arrow_msg.timepoint_max.is_static(),
                )
            }
            LogMsg::SetStoreInfo(store_info) => {
                format!(
                    r#"  {{
    "type": "SetStoreInfo",
    "store_id": "{}"
  }}"#,
                    store_info.info.store_id,
                )
            }
            LogMsg::BlueprintActivationCommand(cmd) => {
                format!(
                    r#"  {{
    "type": "BlueprintActivationCommand",
    "blueprint_id": "{}"
  }}"#,
                    cmd.blueprint_id,
                )
            }
        };
        
        write!(output, "{}", json_str)?;
    }
    
    writeln!(output)?;
    writeln!(output, "]")?;
    output.flush()?;

    Ok(())
}

fn write_csv_output(
    store_info_messages: Vec<LogMsg>,
    messages: Vec<LogMsg>,
    path_to_output_rrd: Option<&String>,
) -> anyhow::Result<()> {
    let mut output: Box<dyn std::io::Write> = if let Some(path) = path_to_output_rrd {
        Box::new(std::io::BufWriter::new(
            std::fs::File::create(path).with_context(|| format!("{path:?}"))?,
        ))
    } else {
        Box::new(std::io::BufWriter::new(std::io::stdout().lock()))
    };

    // CSV header
    writeln!(output, "message_type,chunk_id,timepoint,num_rows,is_static")?;
    
    // Combine StoreInfo messages first, then the extracted messages
    let all_messages = store_info_messages.into_iter().chain(messages.into_iter());
    
    for msg in all_messages {
        match msg {
            LogMsg::ArrowMsg(_id, arrow_msg) => {
                writeln!(
                    output,
                    "ArrowMsg,{},{},{},{}",
                    arrow_msg.chunk_id,
                    format!("{:?}", arrow_msg.timepoint_max).replace(",", ";"), // Escape commas
                    arrow_msg.batch.num_rows(),
                    arrow_msg.timepoint_max.is_static(),
                )?;
            }
            LogMsg::SetStoreInfo(store_info) => {
                writeln!(
                    output,
                    "SetStoreInfo,{},,,0,true",
                    store_info.info.store_id,
                )?;
            }
            LogMsg::BlueprintActivationCommand(cmd) => {
                writeln!(
                    output,
                    "BlueprintActivationCommand,{},,0,true",
                    cmd.blueprint_id,
                )?;
            }
        }
    }
    
    output.flush()?;

    Ok(())
}