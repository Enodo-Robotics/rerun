//! The `rerun-logger` binary - a standalone data logger that buffers and saves Rerun data to .rrd files
//!
//! Run `rerun-logger --help` for more information.

use anyhow::{Context, Result};
use clap::Parser;
use rerun_logger::{parse_byte_size, parse_duration, LoggerConfig, RerunLogger};
use std::{path::PathBuf, time::Duration};

/// Standalone Rerun data logger that buffers and saves data to .rrd files
#[derive(Parser, Debug)]
#[command(
    name = "rerun-logger",
    about = "Standalone Rerun data logger that buffers and saves data to .rrd files",
    long_about = "
The Rerun Logger is a standalone application that extracts the data buffering 
and periodic file saving functionality from the Rerun viewer. It accepts data 
via gRPC from Rerun SDKs and efficiently saves it to .rrd files.

Environment Variables:
  RERUN_FLUSH_TICK_SECS      Flush frequency in seconds (default: 0.008)
  RERUN_FLUSH_NUM_BYTES      Flush threshold in bytes (default: 1048576)
  RERUN_FLUSH_NUM_ROWS       Flush threshold in number of rows (default: unlimited)
  RERUN_LOGGER_MAX_MEMORY    Maximum memory usage before forced flush
  RERUN_LOGGER_PORT          gRPC server port (default: 9876)

Examples:
  # Basic usage - start server mode
  rerun-logger --output data.rrd

  # Connect to external gRPC proxy (like rerun viewer)
  rerun-logger --output data.rrd --connect rerun+http://127.0.0.1:9876/proxy

  # Connect with default proxy URL
  rerun-logger --output data.rrd --connect

  # High-frequency logging with custom thresholds
  rerun-logger --output high_freq.rrd --flush-interval 1s --flush-bytes 10MB

  # Memory-constrained environment
  rerun-logger --output constrained.rrd --max-memory 256MB
",
    version
)]
struct Args {
    /// Output .rrd file path
    #[arg(short, long, value_name = "FILE")]
    output: PathBuf,

    /// gRPC server port to listen on (ignored if --connect is used)
    #[arg(short, long, default_value = "9876")]
    port: u16,

    /// Connect to external gRPC proxy instead of hosting own server
    /// (e.g., rerun+http://127.0.0.1:9876/proxy)
    #[arg(long)]
    #[allow(clippy::option_option)]
    connect: Option<Option<String>>,

    /// Flush interval (e.g., 50ms, 1s, 100us)
    #[arg(long, value_name = "DURATION", default_value = "8ms")]
    flush_interval: String,

    /// Flush when buffer reaches this size (e.g., 1MB, 512KB, 2GB)
    #[arg(long, value_name = "BYTES", default_value = "1MB")]
    flush_bytes: String,

    /// Flush when buffer reaches this many rows
    #[arg(long, value_name = "ROWS")]
    flush_rows: Option<u64>,

    /// Maximum memory usage before forced flush (e.g., 512MB, 1GB)
    #[arg(long, value_name = "BYTES")]
    max_memory: Option<String>,

    /// Compression level for .rrd files
    #[arg(long, default_value = "fast")]
    compression: rerun_logger::CompressionLevel,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    /// Quiet mode (errors only)
    #[arg(short, long)]
    quiet: bool,

    /// Show statistics every N seconds (0 to disable)
    #[arg(long, default_value = "10")]
    stats_interval: u64,

    /// Exit after N seconds (for testing, 0 to run indefinitely)
    #[arg(long, default_value = "0")]
    timeout: u64,
}

impl Args {
    /// Convert CLI arguments to LoggerConfig
    fn to_config(self) -> Result<LoggerConfig> {
        let flush_interval = parse_duration(&self.flush_interval)
            .with_context(|| format!("Invalid flush interval: {}", self.flush_interval))?;
        
        let flush_bytes = parse_byte_size(&self.flush_bytes)
            .with_context(|| format!("Invalid flush bytes: {}", self.flush_bytes))?;

        let max_memory = self.max_memory
            .map(|s| parse_byte_size(&s))
            .transpose()
            .with_context(|| "Invalid max memory size")?;

        // Handle connect option
        let connect_url = match self.connect {
            None => None,
            Some(None) => Some("rerun+http://127.0.0.1:9876/proxy".to_string()), // Default URL
            Some(Some(url)) => Some(url),
        };

        Ok(LoggerConfig {
            output_path: self.output,
            port: self.port,
            connect_url,
            flush_interval,
            flush_bytes,
            flush_rows: self.flush_rows,
            max_memory,
            compression: self.compression,
            verbose: self.verbose,
            quiet: self.quiet,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    // Save values we need before moving args
    let stats_interval = args.stats_interval;
    let timeout = args.timeout;
    
    // Set up logging - re_log will handle environment variables
    re_log::setup_logging();

    // Convert args to config, merging with environment variables
    let mut config = LoggerConfig::from_env()
        .context("Failed to load configuration from environment")?;
    
    // Override with CLI arguments
    let cli_config = args.to_config()
        .context("Failed to parse CLI arguments")?;
    
    // CLI args take precedence over environment variables
    config.output_path = cli_config.output_path;
    config.port = cli_config.port;
    config.connect_url = cli_config.connect_url;
    config.flush_interval = cli_config.flush_interval;
    config.flush_bytes = cli_config.flush_bytes;
    if cli_config.flush_rows.is_some() {
        config.flush_rows = cli_config.flush_rows;
    }
    if cli_config.max_memory.is_some() {
        config.max_memory = cli_config.max_memory;
    }
    config.compression = cli_config.compression;
    config.verbose = cli_config.verbose;
    config.quiet = cli_config.quiet;

    // Validate final configuration
    config.validate()
        .context("Configuration validation failed")?;

    // Create and start the logger
    let logger = RerunLogger::new(config)
        .await
        .context("Failed to create logger")?;

    // Start statistics reporting if enabled
    let stats_handle = if stats_interval > 0 {
        let stats = logger.stats_arc();
        let interval = Duration::from_secs(stats_interval);
        Some(tokio::spawn(async move {
            report_statistics(stats, interval).await;
        }))
    } else {
        None
    };

    // Set up graceful shutdown handling
    let shutdown_signal = {
        let logger_shutdown = logger.shutdown_signal.clone();
        tokio::spawn(async move {
            rerun_logger::server::wait_for_signal().await;
            logger_shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
        })
    };

    // Set up timeout if specified  
    let timeout_handle = if timeout > 0 {
        let logger_shutdown = logger.shutdown_signal.clone();
        let timeout_duration = Duration::from_secs(timeout);
        Some(tokio::spawn(async move {
            tokio::time::sleep(timeout_duration).await;
            re_log::info!("Timeout reached, shutting down");
            logger_shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
        }))
    } else {
        None
    };

    // Run the logger
    let logger_result = logger.run().await;

    // Clean up background tasks
    if let Some(handle) = stats_handle {
        handle.abort();
    }
    if let Some(handle) = timeout_handle {
        handle.abort();
    }
    shutdown_signal.abort();

    // Report final statistics
    let stats = logger.stats();
    re_log::info!("Final statistics:");
    re_log::info!("  Messages received: {}", stats.messages_received.load(std::sync::atomic::Ordering::Relaxed));
    re_log::info!("  Messages written: {}", stats.messages_written.load(std::sync::atomic::Ordering::Relaxed));
    re_log::info!("  Bytes written: {}", stats.bytes_written.load(std::sync::atomic::Ordering::Relaxed));
    re_log::info!("  Flush operations: {}", stats.flush_count.load(std::sync::atomic::Ordering::Relaxed));
    re_log::info!("  Uptime: {:?}", stats.uptime());
    re_log::info!("  Average msg/sec: {:.1}", stats.messages_per_second());
    re_log::info!("  Average bytes/sec: {:.1}", stats.bytes_per_second());

    logger_result.context("Logger execution failed")
}

/// Report statistics periodically
async fn report_statistics(stats: std::sync::Arc<rerun_logger::LoggerStats>, interval: Duration) {
    let mut interval_timer = tokio::time::interval(interval);
    interval_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        interval_timer.tick().await;
        
        let messages_received = stats.messages_received.load(std::sync::atomic::Ordering::Relaxed);
        let messages_written = stats.messages_written.load(std::sync::atomic::Ordering::Relaxed);
        let bytes_written = stats.bytes_written.load(std::sync::atomic::Ordering::Relaxed);
        let memory_usage = stats.memory_usage.load(std::sync::atomic::Ordering::Relaxed);
        let flush_count = stats.flush_count.load(std::sync::atomic::Ordering::Relaxed);

        re_log::info!(
            "Stats: {} msgs recv, {} msgs written, {} bytes written, {} flushes, {:.1} MB memory, {:.1} msg/s, {:.1} B/s",
            messages_received,
            messages_written,
            bytes_written,
            flush_count,
            memory_usage as f64 / 1_000_000.0,
            stats.messages_per_second(),
            stats.bytes_per_second()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_args_parsing() {
        let temp_dir = tempdir().unwrap();
        let output_path = temp_dir.path().join("test.rrd");
        
        let args = Args {
            output: output_path.clone(),
            port: 8877,
            connect: None,
            flush_interval: "100ms".to_string(),
            flush_bytes: "2MB".to_string(),
            flush_rows: Some(1000),
            max_memory: Some("512MB".to_string()),
            compression: rerun_logger::CompressionLevel::Fast,
            verbose: true,
            quiet: false,
            stats_interval: 5,
            timeout: 0,
        };

        let config = args.to_config().unwrap();
        assert_eq!(config.output_path, output_path);
        assert_eq!(config.port, 8877);
        assert_eq!(config.flush_interval, Duration::from_millis(100));
        assert_eq!(config.flush_bytes, 2_000_000);
        assert_eq!(config.flush_rows, Some(1000));
        assert_eq!(config.max_memory, Some(512_000_000));
        assert!(config.verbose);
        assert!(!config.quiet);
    }

    #[test]
    fn test_duration_parsing() {
        assert_eq!(parse_duration("50ms").unwrap(), Duration::from_millis(50));
        assert_eq!(parse_duration("1.5s").unwrap(), Duration::from_secs_f64(1.5));
        assert_eq!(parse_duration("100").unwrap(), Duration::from_millis(100));
    }

    #[test]
    fn test_byte_size_parsing() {
        assert_eq!(parse_byte_size("1MB").unwrap(), 1_000_000);
        assert_eq!(parse_byte_size("512KB").unwrap(), 512_000);
        assert_eq!(parse_byte_size("2GB").unwrap(), 2_000_000_000);
        assert_eq!(parse_byte_size("1024").unwrap(), 1024);
    }
}