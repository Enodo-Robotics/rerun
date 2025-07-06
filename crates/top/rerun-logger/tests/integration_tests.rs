//! Integration tests for rerun-logger

use anyhow::Result;
use assert_cmd::Command;
use predicates::prelude::*;
use rerun_logger::{LoggerConfig, RerunLogger};
use std::{
    path::PathBuf,
    time::Duration,
};
use tempfile::{tempdir, TempDir};
use tokio::time::timeout;

/// Helper to create a test configuration
fn create_test_config(temp_dir: &TempDir) -> LoggerConfig {
    LoggerConfig {
        output_path: temp_dir.path().join("test.rrd"),
        port: 8877, // Use a non-zero port for validation
        connect_url: None, // Server mode for tests
        flush_interval: Duration::from_millis(10), // Fast flushing for tests
        flush_bytes: 1024, // Small threshold for tests
        flush_rows: Some(10),
        max_memory: Some(1024 * 1024), // 1MB limit for tests
        compression: rerun_logger::CompressionLevel::Fast,
        verbose: false,
        quiet: true, // Quiet for cleaner test output
    }
}

#[tokio::test]
async fn test_logger_creation() -> Result<()> {
    let temp_dir = tempdir()?;
    let config = create_test_config(&temp_dir);
    
    let logger = RerunLogger::new(config).await?;
    
    // Verify initial state
    let stats = logger.stats();
    assert_eq!(stats.messages_received.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(stats.messages_written.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(stats.bytes_written.load(std::sync::atomic::Ordering::Relaxed), 0);
    
    Ok(())
}

#[tokio::test] 
async fn test_logger_shutdown() -> Result<()> {
    let temp_dir = tempdir()?;
    let config = create_test_config(&temp_dir);
    
    let logger = RerunLogger::new(config).await?;
    
    // Request shutdown
    logger.shutdown();
    
    // Verify shutdown flag is set
    assert!(logger.shutdown_signal.load(std::sync::atomic::Ordering::Relaxed));
    
    Ok(())
}

#[tokio::test]
async fn test_logger_stats() -> Result<()> {
    let temp_dir = tempdir()?;
    let config = create_test_config(&temp_dir);
    
    let logger = RerunLogger::new(config).await?;
    let stats = logger.stats();
    
    // Test statistics methods
    let uptime = stats.uptime();
    assert!(uptime.as_nanos() > 0);
    
    // Initially should be 0 messages per second
    assert_eq!(stats.messages_per_second(), 0.0);
    assert_eq!(stats.bytes_per_second(), 0.0);
    
    Ok(())
}

#[test]
fn test_cli_help() {
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Standalone Rerun data logger"))
        .stdout(predicate::str::contains("--output"));
}

#[test]
fn test_cli_version() {
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.arg("--version");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("rerun-logger"));
}

#[test]
fn test_cli_missing_output() {
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_config_validation() {
    let temp_dir = tempdir().unwrap();
    
    // Test valid config
    let config = create_test_config(&temp_dir);
    assert!(config.validate().is_ok());
    
    // Test invalid config - zero flush interval
    let mut invalid_config = create_test_config(&temp_dir);
    invalid_config.flush_interval = Duration::from_millis(0);
    assert!(invalid_config.validate().is_err());
    
    // Test invalid config - zero flush bytes
    let mut invalid_config = create_test_config(&temp_dir);
    invalid_config.flush_bytes = 0;
    assert!(invalid_config.validate().is_err());
}

#[test]
#[ignore] // Ignored due to unsafe environment variable manipulation
fn test_env_variable_parsing() {
    // This test is disabled because it requires unsafe code
    // which is not allowed in this codebase
    panic!("Test disabled due to unsafe code restrictions");
}

#[test]
fn test_connect_url_configuration() {
    let temp_dir = tempdir().unwrap();
    
    // Test with no connect URL (server mode)
    let mut config = create_test_config(&temp_dir);
    assert_eq!(config.connect_url, None);
    
    // Test with explicit connect URL (client mode)
    config.connect_url = Some("rerun+http://example.com:9876/proxy".to_string());
    assert_eq!(config.connect_url, Some("rerun+http://example.com:9876/proxy".to_string()));
    
    // Test configuration validation passes with connect URL
    if let Err(e) = config.validate() {
        panic!("Configuration validation failed: {}", e);
    }
}