//! Configuration tests for rerun-logger
//! 
//! Note: Many tests that require environment variable manipulation
//! have been disabled due to unsafe code restrictions in this codebase.

use rerun_logger::{parse_byte_size, parse_duration, CompressionLevel, LoggerConfig};
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn test_parse_duration_units() {
    // Test milliseconds
    assert_eq!(parse_duration("100ms").unwrap(), Duration::from_millis(100));
    assert_eq!(parse_duration("50ms").unwrap(), Duration::from_millis(50));
    
    // Test seconds
    assert_eq!(parse_duration("1s").unwrap(), Duration::from_secs(1));
    assert_eq!(parse_duration("2.5s").unwrap(), Duration::from_secs_f64(2.5));
    
    // Test microseconds
    assert_eq!(parse_duration("500us").unwrap(), Duration::from_micros(500));
    
    // Test raw numbers (treated as milliseconds)
    assert_eq!(parse_duration("100").unwrap(), Duration::from_millis(100));
    
    // Test fractional seconds
    assert_eq!(parse_duration("0.1").unwrap(), Duration::from_secs_f64(0.1));
}

#[test]
fn test_parse_duration_errors() {
    assert!(parse_duration("").is_err());
    assert!(parse_duration("invalid").is_err());
    assert!(parse_duration("-100ms").is_err());
    assert!(parse_duration("abc123").is_err());
}

#[test]
fn test_parse_byte_size_units() {
    // Test bytes
    assert_eq!(parse_byte_size("1024").unwrap(), 1024);
    assert_eq!(parse_byte_size("512").unwrap(), 512);
    
    // Test KB (1000-based)
    assert_eq!(parse_byte_size("1KB").unwrap(), 1000);
    assert_eq!(parse_byte_size("2KB").unwrap(), 2000);
    
    // Test MB
    assert_eq!(parse_byte_size("1MB").unwrap(), 1_000_000);
    assert_eq!(parse_byte_size("10MB").unwrap(), 10_000_000);
    
    // Test GB
    assert_eq!(parse_byte_size("1GB").unwrap(), 1_000_000_000);
    assert_eq!(parse_byte_size("2GB").unwrap(), 2_000_000_000);
    
    // Test KiB (1024-based)
    assert_eq!(parse_byte_size("1KiB").unwrap(), 1024);
    assert_eq!(parse_byte_size("2KiB").unwrap(), 2048);
    
    // Test MiB
    assert_eq!(parse_byte_size("1MiB").unwrap(), 1024 * 1024);
    assert_eq!(parse_byte_size("10MiB").unwrap(), 10 * 1024 * 1024);
    
    // Test GiB
    assert_eq!(parse_byte_size("1GiB").unwrap(), 1024 * 1024 * 1024);
}

#[test]
fn test_parse_byte_size_errors() {
    assert!(parse_byte_size("").is_err());
    assert!(parse_byte_size("invalid").is_err());
    assert!(parse_byte_size("-100MB").is_err());
    assert!(parse_byte_size("100XB").is_err());
    assert!(parse_byte_size("abc").is_err());
}

#[test]
fn test_default_config() {
    let config = LoggerConfig::default();
    
    // Test default values
    assert_eq!(config.flush_interval, Duration::from_secs_f64(0.008));
    assert_eq!(config.flush_bytes, 1048576); // 1MB
    assert_eq!(config.flush_rows, None);
    assert_eq!(config.max_memory, None);
    assert_eq!(config.port, 9876);
    assert!(!config.verbose);
    assert!(!config.quiet);
    assert!(matches!(config.compression, CompressionLevel::Fast));
}

#[test]
fn test_config_validation() {
    let temp_dir = tempdir().unwrap();
    
    // Valid config
    let config = LoggerConfig {
        output_path: temp_dir.path().join("test.rrd"),
        port: 8877,
        connect_url: None,
        flush_interval: Duration::from_millis(100),
        flush_bytes: 1024,
        flush_rows: Some(100),
        max_memory: Some(1024 * 1024),
        compression: CompressionLevel::Fast,
        verbose: false,
        quiet: false,
    };
    assert!(config.validate().is_ok());
    
    // Invalid: zero flush interval  
    let mut invalid_config = config.clone();
    invalid_config.flush_interval = Duration::from_millis(0);
    assert!(invalid_config.validate().is_err());
    
    // Invalid: zero flush bytes
    let mut invalid_config = config.clone();
    invalid_config.flush_bytes = 0;
    assert!(invalid_config.validate().is_err());
    
    // Invalid: both verbose and quiet
    let mut invalid_config = config.clone();
    invalid_config.verbose = true;
    invalid_config.quiet = true;
    assert!(invalid_config.validate().is_err());
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
#[ignore] // Environment variable tests disabled due to unsafe code restrictions
fn test_env_parsing_disabled() {
    // These tests are disabled because they require unsafe environment variable manipulation
    // which is not allowed in this codebase
}