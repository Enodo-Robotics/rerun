//! Rerun Logger - A standalone data logger that buffers and saves Rerun data to .rrd files
//!
//! This crate provides functionality to:
//! - Buffer incoming Rerun log messages in memory
//! - Periodically flush data to .rrd files based on configurable thresholds
//! - Accept data via gRPC server from Rerun SDKs
//! - Manage memory usage and perform garbage collection

use re_memory::AccountingAllocator;

#[global_allocator]
static GLOBAL: AccountingAllocator<mimalloc::MiMalloc> =
    AccountingAllocator::new(mimalloc::MiMalloc);

pub mod client;
pub mod config;
pub mod logger;
pub mod server;

pub use client::run_grpc_client;
pub use config::*;
pub use logger::*;
pub use server::run_grpc_server;

use anyhow::Result;
use std::time::Duration;

/// Main entry point for the rerun-logger application
pub async fn run_logger(config: LoggerConfig) -> Result<()> {
    re_log::setup_logging();
    
    let logger = RerunLogger::new(config).await?;
    logger.run().await
}

/// Parse a duration string (e.g., "50ms", "1s", "100us")
pub fn parse_duration(s: &str) -> Result<Duration> {
    if s.ends_with("ms") {
        let ms: u64 = s[..s.len() - 2].parse()?;
        Ok(Duration::from_millis(ms))
    } else if s.ends_with("us") {
        let us: u64 = s[..s.len() - 2].parse()?;
        Ok(Duration::from_micros(us))
    } else if s.ends_with("ns") {
        let ns: u64 = s[..s.len() - 2].parse()?;
        Ok(Duration::from_nanos(ns))
    } else if s.ends_with('s') {
        let secs: f64 = s[..s.len() - 1].parse()?;
        Ok(Duration::from_secs_f64(secs))
    } else {
        // Try parsing as milliseconds if no unit specified
        let ms_f64: f64 = s.parse()?;
        Ok(Duration::from_secs_f64(ms_f64 / 1000.0))
    }
}

/// Parse a byte size string (e.g., "1MB", "512KB", "2GB")
pub fn parse_byte_size(s: &str) -> Result<u64> {
    let s = s.to_uppercase();
    
    if s.ends_with("GB") {
        let gb: f64 = s[..s.len() - 2].parse()?;
        Ok((gb * 1_000_000_000.0) as u64)
    } else if s.ends_with("MB") {
        let mb: f64 = s[..s.len() - 2].parse()?;
        Ok((mb * 1_000_000.0) as u64)
    } else if s.ends_with("KB") {
        let kb: f64 = s[..s.len() - 2].parse()?;
        Ok((kb * 1_000.0) as u64)
    } else if s.ends_with('B') {
        let bytes: u64 = s[..s.len() - 1].parse()?;
        Ok(bytes)
    } else {
        // Try parsing as raw bytes if no unit specified
        Ok(s.parse()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("100ms").unwrap(), Duration::from_millis(100));
        assert_eq!(parse_duration("1s").unwrap(), Duration::from_secs(1));
        assert_eq!(parse_duration("1.5s").unwrap(), Duration::from_secs_f64(1.5));
        assert_eq!(parse_duration("500us").unwrap(), Duration::from_micros(500));
        assert_eq!(parse_duration("50").unwrap(), Duration::from_secs_f64(0.05));
    }

    #[test]
    fn test_parse_byte_size() {
        assert_eq!(parse_byte_size("1MB").unwrap(), 1_000_000);
        assert_eq!(parse_byte_size("1.5MB").unwrap(), 1_500_000);
        assert_eq!(parse_byte_size("512KB").unwrap(), 512_000);
        assert_eq!(parse_byte_size("2GB").unwrap(), 2_000_000_000);
        assert_eq!(parse_byte_size("1024B").unwrap(), 1024);
        assert_eq!(parse_byte_size("1024").unwrap(), 1024);
    }
}