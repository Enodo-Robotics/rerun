use std::io::{IsTerminal as _, Write as _};

use anyhow::Context as _;
use itertools::Either;

use re_chunk_store::ChunkStoreConfig;
use re_entity_db::EntityDb;
use re_log_types::{LogMsg, ResolvedTimeRangeF, StoreId, TimeInt};
use re_chunk::TimelineName;
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
pub struct SpliceCommand {
    /// Paths to read from. Reads from standard input if none are specified.
    path_to_input_rrds: Vec<String>,

    /// Path to write to. Writes to standard output if unspecified.
    #[arg(short = 'o', long = "output", value_name = "dst.(rrd|rbl)")]
    path_to_output_rrd: Option<String>,

    /// Timeline to use for splicing (e.g., log_time, log_tick, frame, time).
    #[arg(short = 't', long = "timeline", default_value = "log_time")]
    timeline: String,

    /// Start time for the splice (inclusive). Can be nanoseconds, seconds, or frame numbers.
    #[arg(long = "start")]
    start_time: Option<i64>,

    /// End time for the splice (exclusive). Can be nanoseconds, seconds, or frame numbers.
    #[arg(long = "end")]
    end_time: Option<i64>,

    /// Exclude static (timeline-less) data from the output. By default, static data is included.
    #[arg(long = "exclude-static")]
    exclude_static: bool,

    /// Include only static (timeline-less) data in the output. Cannot be used with --exclude-static.
    #[arg(long = "static-only")]
    static_only: bool,

    /// If set, will try to proceed even in the face of IO and/or decoding errors in the input data.
    #[clap(long = "continue-on-error", default_value_t = false)]
    continue_on_error: bool,
}

impl SpliceCommand {
    pub fn run(&self) -> anyhow::Result<()> {
        let Self {
            path_to_input_rrds,
            path_to_output_rrd,
            timeline,
            start_time,
            end_time,
            exclude_static,
            static_only,
            continue_on_error,
        } = self;

        if path_to_output_rrd.is_none() {
            anyhow::ensure!(
                !std::io::stdout().is_terminal(),
                "you must redirect the output to a file and/or stream"
            );
        }

        // Validate time range
        if let (Some(start), Some(end)) = (start_time, end_time) {
            anyhow::ensure!(
                start < end,
                "start time ({}) must be less than end time ({})",
                start,
                end
            );
        }

        // Validate conflicting options
        if *exclude_static && *static_only {
            anyhow::bail!("Cannot specify both --exclude-static and --static-only");
        }

        // Parse timeline
        let timeline_name = TimelineName::new(timeline.as_str());
        
        // Build time range
        let time_selection = match (start_time, end_time) {
            (Some(start), Some(end)) => Some((
                timeline_name,
                ResolvedTimeRangeF::new(
                    TimeInt::new_temporal(*start),
                    TimeInt::new_temporal(*end),
                ),
            )),
            (Some(start), None) => Some((
                timeline_name,
                ResolvedTimeRangeF::new(
                    TimeInt::new_temporal(*start),
                    TimeInt::MAX,
                ),
            )),
            (None, Some(end)) => Some((
                timeline_name,
                ResolvedTimeRangeF::new(
                    TimeInt::MIN,
                    TimeInt::new_temporal(*end),
                ),
            )),
            (None, None) => None, // No filtering - return everything
        };

        // NOTE #1: We're doing headless processing, there's no point in running subscribers, it will just
        // (massively) slow us down.
        // NOTE #2: We do not want to modify the configuration of the original data in any way
        // (e.g. by recompacting it differently), so make sure to disable all these features.
        let store_config = ChunkStoreConfig::ALL_DISABLED;

        splice_recording(
            *continue_on_error,
            &store_config,
            path_to_input_rrds,
            path_to_output_rrd.as_ref(),
            time_selection,
            *exclude_static,
            *static_only,
        )
    }
}

fn splice_recording(
    continue_on_error: bool,
    store_config: &ChunkStoreConfig,
    path_to_input_rrds: &[String],
    path_to_output_rrd: Option<&String>,
    time_selection: Option<(TimelineName, ResolvedTimeRangeF)>,
    exclude_static: bool,
    static_only: bool,
) -> anyhow::Result<()> {
    let file_size_to_string = |size: Option<u64>| {
        size.map_or_else(
            || "<unknown>".to_owned(),
            |size| re_format::format_bytes(size as _),
        )
    };

    let now = std::time::Instant::now();
    
    let time_info = match &time_selection {
        Some((timeline_name, time_range)) => format!(
            "timeline '{}' from {} to {}",
            timeline_name,
            time_range.min.as_f64(),
            time_range.max.as_f64()
        ),
        None => "no time filtering".to_owned(),
    };

    re_log::info!(
        srcs = ?path_to_input_rrds,
        time_selection = %time_info,
        "splice started"
    );

    let (rx, rx_size_bytes) = read_rrd_streams_from_file_or_stdin(path_to_input_rrds);

    let mut entity_dbs: std::collections::HashMap<StoreId, EntityDb> = Default::default();

    for (_source, res) in rx {
        let mut is_success = true;

        match res {
            Ok(msg) => {
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

    let mut rrd_out = if let Some(path) = path_to_output_rrd {
        Either::Left(std::io::BufWriter::new(
            std::fs::File::create(path).with_context(|| format!("{path:?}"))?,
        ))
    } else {
        Either::Right(std::io::BufWriter::new(std::io::stdout().lock()))
    };

    // Apply time selection to recording databases only
    // Blueprints are always included in full
    let messages_rbl = entity_dbs
        .values()
        .filter(|entity_db| entity_db.store_kind() == StoreKind::Blueprint)
        .flat_map(|entity_db| entity_db.to_messages(None /* time selection */))
        .filter_map(|msg_result| match msg_result {
            Ok(msg) => {
                let is_static = is_static_message(&msg);
                if (exclude_static && is_static) || (static_only && !is_static) {
                    None
                } else {
                    Some(Ok(msg))
                }
            }
            Err(err) => Some(Err(err)),
        });

    let messages_rrd = entity_dbs
        .values()
        .filter(|entity_db| entity_db.store_kind() == StoreKind::Recording)
        .flat_map(|entity_db| {
            // Include ALL static messages regardless of time selection
            // Static data represents timeless state and should always be included
            let static_messages = entity_db.to_messages(None)
                .filter_map(|msg_result| match msg_result {
                    Ok(msg) if is_static_message(&msg) && !exclude_static => Some(Ok(msg)),
                    _ => None,
                });
            
            // Include temporal messages within the time selection
            let temporal_messages = entity_db.to_messages(time_selection)
                .filter_map(|msg_result| match msg_result {
                    Ok(msg) if !is_static_message(&msg) && !static_only => Some(Ok(msg)),
                    Err(err) => Some(Err(err)),
                    _ => None,
                });
            
            // Chain static and temporal messages
            static_messages.chain(temporal_messages)
        });

    // TODO(cmc): encoding options should match the original.
    let encoding_options = re_log_encoding::EncodingOptions::PROTOBUF_COMPRESSED;
    let version = entity_dbs
        .values()
        .next()
        .and_then(|db| db.store_info())
        .and_then(|info| info.store_version)
        .unwrap_or(re_build_info::CrateVersion::LOCAL);
    let rrd_out_size = re_log_encoding::encoder::encode(
        version,
        encoding_options,
        // NOTE: We want to make sure all blueprints come first, so that the viewer can immediately
        // set up the viewport correctly.
        messages_rbl.chain(messages_rrd),
        &mut rrd_out,
    )
    .context("couldn't encode messages")?;

    rrd_out.flush().context("couldn't flush output")?;

    let rrds_in_size = rx_size_bytes.recv().ok();
    let size_reduction = if let (Some(rrds_in_size), rrd_out_size) = (rrds_in_size, rrd_out_size) {
        format!(
            "-{:3.3}%",
            100.0 - rrd_out_size as f64 / (rrds_in_size as f64 + f64::EPSILON) * 100.0
        )
    } else {
        "N/A".to_owned()
    };

    re_log::info!(
        dst_size_bytes = %file_size_to_string(Some(rrd_out_size)),
        time = ?now.elapsed(),
        size_reduction,
        srcs = ?path_to_input_rrds,
        srcs_size_bytes = %file_size_to_string(rrds_in_size),
        time_selection = %time_info,
        "splice finished"
    );

    Ok(())
}