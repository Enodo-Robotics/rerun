//! CLI-specific tests

use assert_cmd::Command;
use predicates::prelude::*;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn test_cli_help_contains_expected_options() {
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--output"))
        .stdout(predicate::str::contains("--port"))
        .stdout(predicate::str::contains("--flush-interval"))
        .stdout(predicate::str::contains("--flush-bytes"))
        .stdout(predicate::str::contains("--flush-rows"))
        .stdout(predicate::str::contains("--max-memory"))
        .stdout(predicate::str::contains("--compression"))
        .stdout(predicate::str::contains("--verbose"))
        .stdout(predicate::str::contains("--quiet"));
}

#[test]
fn test_cli_version_output() {
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("rerun-logger"))
        .stdout(predicate::str::is_match(r"\d+\.\d+\.\d+").unwrap());
}

#[test]
fn test_cli_missing_required_output() {
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_cli_basic_valid_args() {
    let temp_dir = tempdir().unwrap();
    let output_path = temp_dir.path().join("test.rrd");
    
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.args(&[
        "--output", output_path.to_str().unwrap(),
        "--timeout", "1", // Exit after 1 second for testing
        "--quiet"
    ])
    .timeout(Duration::from_secs(5))
    .assert()
    .success();
}

#[test]
fn test_cli_custom_port() {
    let temp_dir = tempdir().unwrap();
    let output_path = temp_dir.path().join("test.rrd");
    
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.args(&[
        "--output", output_path.to_str().unwrap(),
        "--port", "8877",
        "--timeout", "1",
        "--quiet"
    ])
    .timeout(Duration::from_secs(5))
    .assert()
    .success();
}

#[test]
fn test_cli_custom_flush_settings() {
    let temp_dir = tempdir().unwrap();
    let output_path = temp_dir.path().join("test.rrd");
    
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.args(&[
        "--output", output_path.to_str().unwrap(),
        "--flush-interval", "100ms",
        "--flush-bytes", "2MB",
        "--flush-rows", "500",
        "--timeout", "1",
        "--quiet"
    ])
    .timeout(Duration::from_secs(5))
    .assert()
    .success();
}

#[test]
fn test_cli_memory_limit() {
    let temp_dir = tempdir().unwrap();
    let output_path = temp_dir.path().join("test.rrd");
    
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.args(&[
        "--output", output_path.to_str().unwrap(),
        "--max-memory", "128MB",
        "--timeout", "1",
        "--quiet"
    ])
    .timeout(Duration::from_secs(5))
    .assert()
    .success();
}

#[test]
fn test_cli_compression_levels() {
    let temp_dir = tempdir().unwrap();
    let output_path = temp_dir.path().join("test.rrd");
    
    for compression in &["none", "fast", "balanced", "high"] {
        let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
        cmd.args(&[
            "--output", output_path.to_str().unwrap(),
            "--compression", compression,
            "--timeout", "1",
            "--quiet"
        ])
        .timeout(Duration::from_secs(5))
        .assert()
        .success();
    }
}

#[test]
fn test_cli_verbose_and_quiet_flags() {
    let temp_dir = tempdir().unwrap();
    let output_path = temp_dir.path().join("test.rrd");
    
    // Test verbose flag
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.args(&[
        "--output", output_path.to_str().unwrap(),
        "--verbose",
        "--timeout", "1"
    ])
    .timeout(Duration::from_secs(5))
    .assert()
    .success();
    
    // Test quiet flag
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.args(&[
        "--output", output_path.to_str().unwrap(),
        "--quiet",
        "--timeout", "1"
    ])
    .timeout(Duration::from_secs(5))
    .assert()
    .success();
}

#[test]
fn test_cli_stats_interval() {
    let temp_dir = tempdir().unwrap();
    let output_path = temp_dir.path().join("test.rrd");
    
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.args(&[
        "--output", output_path.to_str().unwrap(),
        "--stats-interval", "1",
        "--timeout", "2",
        "--quiet"
    ])
    .timeout(Duration::from_secs(5))
    .assert()
    .success();
}

#[test]
fn test_cli_invalid_flush_interval() {
    let temp_dir = tempdir().unwrap();
    let output_path = temp_dir.path().join("test.rrd");
    
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.args(&[
        "--output", output_path.to_str().unwrap(),
        "--flush-interval", "invalid"
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("Invalid flush interval"));
}

#[test]
fn test_cli_invalid_flush_bytes() {
    let temp_dir = tempdir().unwrap();
    let output_path = temp_dir.path().join("test.rrd");
    
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.args(&[
        "--output", output_path.to_str().unwrap(),
        "--flush-bytes", "invalid"
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("Invalid flush bytes"));
}

#[test]
fn test_cli_invalid_max_memory() {
    let temp_dir = tempdir().unwrap();
    let output_path = temp_dir.path().join("test.rrd");
    
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.args(&[
        "--output", output_path.to_str().unwrap(),
        "--max-memory", "invalid"
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("Invalid max memory size"));
}

#[test]
fn test_cli_invalid_compression() {
    let temp_dir = tempdir().unwrap();
    let output_path = temp_dir.path().join("test.rrd");
    
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.args(&[
        "--output", output_path.to_str().unwrap(),
        "--compression", "invalid"
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("invalid value"));
}

#[test]
fn test_cli_invalid_port() {
    let temp_dir = tempdir().unwrap();
    let output_path = temp_dir.path().join("test.rrd");
    
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.args(&[
        "--output", output_path.to_str().unwrap(),
        "--port", "invalid"
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("invalid value"));
}

#[test]
fn test_cli_port_out_of_range() {
    let temp_dir = tempdir().unwrap();
    let output_path = temp_dir.path().join("test.rrd");
    
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.args(&[
        "--output", output_path.to_str().unwrap(),
        "--port", "99999"
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("invalid value"));
}

#[test]
fn test_cli_output_file_creation() {
    let temp_dir = tempdir().unwrap();
    let output_path = temp_dir.path().join("subdir").join("nested").join("test.rrd");
    
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.args(&[
        "--output", output_path.to_str().unwrap(),
        "--timeout", "1",
        "--quiet"
    ])
    .timeout(Duration::from_secs(5))
    .assert()
    .success();
    
    // Check that parent directories were created
    assert!(output_path.parent().unwrap().exists());
}

#[test]
#[ignore] // Ignored due to unsafe environment variable manipulation
fn test_cli_environment_variable_override() {
    // This test is disabled because it requires unsafe code
    // which is not allowed in this codebase
    panic!("Test disabled due to unsafe code restrictions");
}

#[test]
fn test_cli_long_help_content() {
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Environment Variables:"))
        .stdout(predicate::str::contains("RERUN_FLUSH_TICK_SECS"))
        .stdout(predicate::str::contains("Examples:"))
        .stdout(predicate::str::contains("rerun-logger --output"))
        .stdout(predicate::str::contains("--connect"));
}

#[test]
fn test_cli_connect_option_explicit_url() {
    let temp_dir = tempdir().unwrap();
    let output_path = temp_dir.path().join("test.rrd");
    
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.args(&[
        "--output", output_path.to_str().unwrap(),
        "--connect", "rerun+http://example.com:9876/proxy", 
        "--timeout", "1",
        "--quiet"
    ])
    .timeout(Duration::from_secs(5))
    .assert()
    .success(); // Should succeed even if connection fails (graceful handling)
}

#[test]
fn test_cli_connect_option_default_url() {
    let temp_dir = tempdir().unwrap();
    let output_path = temp_dir.path().join("test.rrd");
    
    let mut cmd = Command::cargo_bin("rerun-logger").unwrap();
    cmd.args(&[
        "--output", output_path.to_str().unwrap(),
        "--connect", // No URL specified, should use default
        "--timeout", "1",
        "--quiet"
    ])
    .timeout(Duration::from_secs(5))
    .assert()
    .success(); // Should succeed even if connection fails (graceful handling)
}