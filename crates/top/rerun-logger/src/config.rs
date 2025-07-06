//! Configuration for the Rerun Logger

use anyhow::{Context, Result};
use std::{path::PathBuf, time::Duration};

/// Configuration for the Rerun Logger
#[derive(Debug, Clone)]
pub struct LoggerConfig {
    /// Output file path for .rrd data
    pub output_path: PathBuf,
    
    /// gRPC server port to listen on (ignored if connecting to external proxy)
    pub port: u16,
    
    /// Connect to external gRPC proxy instead of hosting our own server
    pub connect_url: Option<String>,
    
    /// How often to flush data to disk
    pub flush_interval: Duration,
    
    /// Flush when buffer reaches this size in bytes
    pub flush_bytes: u64,
    
    /// Flush when buffer reaches this many rows (optional)
    pub flush_rows: Option<u64>,
    
    /// Maximum memory usage before forced flush (optional)
    pub max_memory: Option<u64>,
    
    /// Compression level for .rrd files
    pub compression: CompressionLevel,
    
    /// Verbose logging
    pub verbose: bool,
    
    /// Quiet mode (errors only)
    pub quiet: bool,
}

/// Compression levels for .rrd files
#[derive(Debug, Clone, Copy)]
pub enum CompressionLevel {
    None,
    Fast,
    Balanced,
    High,
}

impl std::str::FromStr for CompressionLevel {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "none" | "off" => Ok(CompressionLevel::None),
            "fast" | "low" => Ok(CompressionLevel::Fast),
            "balanced" | "medium" => Ok(CompressionLevel::Balanced),
            "high" | "max" => Ok(CompressionLevel::High),
            _ => anyhow::bail!("Invalid compression level: {}", s),
        }
    }
}

impl Default for LoggerConfig {
    fn default() -> Self {
        Self {
            output_path: PathBuf::from("output.rrd"),
            port: 9876,
            connect_url: None,
            flush_interval: Duration::from_millis(8), // Same as ChunkBatcher default
            flush_bytes: 1024 * 1024, // 1 MB
            flush_rows: None, // Unlimited by default
            max_memory: None,
            compression: CompressionLevel::Fast,
            verbose: false,
            quiet: false,
        }
    }
}

impl LoggerConfig {
    /// Create a new LoggerConfig from environment variables
    pub fn from_env() -> Result<Self> {
        let mut config = Self::default();
        
        // Override with environment variables if present
        if let Ok(tick_secs) = std::env::var("RERUN_FLUSH_TICK_SECS") {
            let secs: f64 = tick_secs.parse()
                .context("Failed to parse RERUN_FLUSH_TICK_SECS")?;
            config.flush_interval = Duration::from_secs_f64(secs);
        }
        
        if let Ok(flush_bytes) = std::env::var("RERUN_FLUSH_NUM_BYTES") {
            config.flush_bytes = crate::parse_byte_size(&flush_bytes)
                .context("Failed to parse RERUN_FLUSH_NUM_BYTES")?;
        }
        
        if let Ok(flush_rows) = std::env::var("RERUN_FLUSH_NUM_ROWS") {
            let rows: u64 = flush_rows.parse()
                .context("Failed to parse RERUN_FLUSH_NUM_ROWS")?;
            config.flush_rows = Some(rows);
        }
        
        if let Ok(max_memory) = std::env::var("RERUN_LOGGER_MAX_MEMORY") {
            config.max_memory = Some(crate::parse_byte_size(&max_memory)
                .context("Failed to parse RERUN_LOGGER_MAX_MEMORY")?);
        }
        
        if let Ok(port) = std::env::var("RERUN_LOGGER_PORT") {
            config.port = port.parse()
                .context("Failed to parse RERUN_LOGGER_PORT")?;
        }
        
        Ok(config)
    }
    
    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        if self.port == 0 {
            anyhow::bail!("Port cannot be zero");
        }
        
        if self.flush_interval.is_zero() {
            anyhow::bail!("Flush interval cannot be zero");
        }
        
        if self.flush_bytes == 0 {
            anyhow::bail!("Flush bytes cannot be zero");
        }
        
        if let Some(rows) = self.flush_rows {
            if rows == 0 {
                anyhow::bail!("Flush rows cannot be zero");
            }
        }
        
        // Ensure output directory exists or can be created
        if let Some(parent) = self.output_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create output directory: {}", parent.display()))?;
        }
        
        Ok(())
    }
    
    /// Get the chunk batcher configuration
    pub fn chunk_batcher_config(&self) -> re_chunk::ChunkBatcherConfig {
        re_chunk::ChunkBatcherConfig {
            flush_tick: self.flush_interval,
            flush_num_bytes: self.flush_bytes,
            flush_num_rows: self.flush_rows.unwrap_or(u64::MAX),
            chunk_max_rows_if_unsorted: 256, // Reasonable default
            max_commands_in_flight: None,
            max_chunks_in_flight: None,
            hooks: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = LoggerConfig::default();
        assert_eq!(config.port, 9876);
        assert_eq!(config.flush_interval, Duration::from_millis(8));
        assert_eq!(config.flush_bytes, 1024 * 1024);
        assert!(config.flush_rows.is_none());
    }

    #[test]
    fn test_config_validation() {
        let mut config = LoggerConfig::default();
        
        // Valid config should pass
        assert!(config.validate().is_ok());
        
        // Invalid port
        config.port = 0;
        assert!(config.validate().is_err());
        config.port = 9876;
        
        // Invalid flush interval
        config.flush_interval = Duration::ZERO;
        assert!(config.validate().is_err());
        config.flush_interval = Duration::from_millis(8);
        
        // Invalid flush bytes
        config.flush_bytes = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_compression_level_parsing() {
        assert!(matches!("none".parse::<CompressionLevel>().unwrap(), CompressionLevel::None));
        assert!(matches!("fast".parse::<CompressionLevel>().unwrap(), CompressionLevel::Fast));
        assert!(matches!("balanced".parse::<CompressionLevel>().unwrap(), CompressionLevel::Balanced));
        assert!(matches!("high".parse::<CompressionLevel>().unwrap(), CompressionLevel::High));
        assert!("invalid".parse::<CompressionLevel>().is_err());
    }

    #[test]
    #[ignore] // Ignored due to unsafe environment variable manipulation
    fn test_env_override() {
        // This test is disabled because it requires unsafe code
        // which is not allowed in this codebase
        panic!("Test disabled due to unsafe code restrictions");
    }
}