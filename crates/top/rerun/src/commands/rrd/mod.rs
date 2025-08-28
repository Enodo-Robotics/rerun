mod compare;
mod extract;
mod filter;
mod merge_compact;
mod print;
mod splice;
mod split;
mod verify;

use self::compare::CompareCommand;
use self::extract::ExtractCommand;
use self::filter::FilterCommand;
use self::merge_compact::{CompactCommand, MergeCommand};
use self::print::PrintCommand;
use self::splice::SpliceCommand;
use self::split::SplitCommand;
use self::verify::VerifyCommand;

// ---

use anyhow::Context as _;
use clap::Subcommand;

/// Manipulate the contents of .rrd and .rbl files.
#[derive(Debug, Clone, Subcommand)]
pub enum RrdCommands {
    /// Compares the data between 2 .rrd files, returning a successful shell exit code if they
    /// match.
    ///
    /// This ignores the `log_time` timeline.
    Compare(CompareCommand),

    /// Print the contents of one or more .rrd/.rbl files/streams.
    ///
    /// Reads from standard input if no paths are specified.
    ///
    /// Example: `rerun rrd print /my/recordings/*.rrd`
    Print(PrintCommand),

    /// Verify the that the .rrd file can be loaded and correctly interpreted.
    ///
    /// Can be used to ensure that the current Rerun version can load the data.
    Verify(VerifyCommand),

    /// Compacts the contents of one or more .rrd/.rbl files/streams and writes the result standard output.
    ///
    /// Reads from standard input if no paths are specified.
    ///
    /// Uses the usual environment variables to control the compaction thresholds:
    /// `RERUN_CHUNK_MAX_ROWS`,
    /// `RERUN_CHUNK_MAX_ROWS_IF_UNSORTED`,
    /// `RERUN_CHUNK_MAX_BYTES`.
    ///
    /// Unless explicit flags are passed, in which case they will override environment values.
    ///
    /// Examples:
    ///
    /// * `RERUN_CHUNK_MAX_ROWS=4096 RERUN_CHUNK_MAX_BYTES=1048576 rerun rrd compact /my/recordings/*.rrd -o output.rrd`
    ///
    /// * `rerun rrd compact --max-rows 4096 --max-bytes=1048576 /my/recordings/*.rrd > output.rrd`
    Compact(CompactCommand),

    /// Merges the contents of multiple .rrd/.rbl files/streams, and writes the result to standard output.
    ///
    /// Reads from standard input if no paths are specified.
    ///
    /// This will not affect the chunking of the data in any way.
    ///
    /// Example: `rerun merge /my/recordings/*.rrd > output.rrd`
    Merge(MergeCommand),

    /// Extracts and analyzes data from .rrd/.rbl files/streams with filtering options.
    ///
    /// Reads from standard input if no paths are specified.
    ///
    /// Supports extraction of static data, temporal data, specific entity paths, and multiple output formats.
    /// Always includes data from all timelines when extracting temporal data.
    ///
    /// Examples:
    /// * `rerun rrd extract --static-only /my/recordings/*.rrd -o static.rrd`
    /// * `rerun rrd extract --entity-path '/world/**' /my/recordings/*.rrd -o world_data.rrd`
    /// * `rerun rrd extract --entity-path '/cameras/*' --entity-path '/radar/*' /my/recordings/*.rrd -o sensors.rrd`
    /// * `rerun rrd extract --entity-path '/cameras/*' --format json /my/recordings/*.rrd -o cameras.json`
    /// * `rerun rrd extract --list-entities /my/recordings/*.rrd`
    Extract(ExtractCommand),

    /// Filters out data from .rrd/.rbl files/streams, and writes the result to standard output.
    ///
    /// Reads from standard input if no paths are specified.
    ///
    /// This will not affect the chunking of the data in any way.
    ///
    /// Example: `rerun filter --drop-timeline log_tick /my/recordings/*.rrd > output.rrd`
    Filter(FilterCommand),

    /// Extracts a time-based slice from .rrd/.rbl files/streams, and writes the result to standard output.
    ///
    /// Reads from standard input if no paths are specified.
    ///
    /// This allows you to extract data within a specific time range on a specified timeline.
    ///
    /// Example: `rerun rrd splice --timeline log_time --start 1000000000 --end 2000000000 /my/recordings/*.rrd -o output.rrd`
    Splice(SpliceCommand),

    /// Splits .rrd/.rbl files into smaller chunks of a specified maximum size.
    ///
    /// Reads from standard input if no paths are specified.
    ///
    /// The split files can be recombined using `rerun rrd merge` to reconstruct the original.
    /// A merge script is automatically generated for convenience.
    ///
    /// Example: `rerun rrd split --size 50MB --output-dir ./chunks /my/recordings/*.rrd`
    Split(SplitCommand),
}

impl RrdCommands {
    pub fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::Compare(compare_command) => {
                compare_command
                    .run()
                    // Print current directory, this can be useful for debugging issues with relative paths.
                    .with_context(|| format!("current directory {:?}", std::env::current_dir()))
            }
            Self::Extract(extract_command) => extract_command.run(),
            Self::Print(print_command) => print_command.run(),
            Self::Verify(verify_command) => verify_command.run(),
            Self::Compact(compact_command) => compact_command.run(),
            Self::Merge(merge_command) => merge_command.run(),
            Self::Filter(drop_command) => drop_command.run(),
            Self::Splice(splice_command) => splice_command.run(),
            Self::Split(split_command) => split_command.run(),
        }
    }
}
