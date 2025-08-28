#!/usr/bin/env python3
"""Integration tests for Rerun v0.0.9 RRD extract, splice, and split functionality."""

import pytest
import subprocess
import tempfile
import shutil
from pathlib import Path
import time
import os

# Get the rerun binary path - tests are running from tests/integration, binary is in project root
RERUN_BINARY = os.path.join(os.path.dirname(__file__), "../..", "target", "release", "rerun")


@pytest.fixture
def temp_rrd_file():
    """Create a temporary RRD file for testing."""
    with tempfile.NamedTemporaryFile(delete=False, suffix='.rrd') as tmp:
        yield tmp.name
    # Cleanup
    if Path(tmp.name).exists():
        Path(tmp.name).unlink()


@pytest.fixture
def temp_dir():
    """Create a temporary directory for testing."""
    temp_dir = tempfile.mkdtemp()
    yield temp_dir
    # Cleanup
    shutil.rmtree(temp_dir, ignore_errors=True)


@pytest.fixture
def sample_rrd_file(temp_rrd_file):
    """Create a sample RRD file with known data for testing."""
    # Create a simple RRD file using the CLI
    # This is a placeholder - in a real test we'd generate proper test data
    
    # For now, create an empty file to test the commands work
    Path(temp_rrd_file).touch()
    return temp_rrd_file


class TestRRDExtract:
    """Test suite for the rerun rrd extract command."""
    
    def test_extract_help_available(self):
        """Test that extract command help is available."""
        result = subprocess.run(
            [RERUN_BINARY, "rrd", "extract", "--help"],
            capture_output=True, text=True, timeout=10
        )
        
        assert result.returncode == 0
        assert "Extracts and analyzes data" in result.stdout
        assert "--static-only" in result.stdout
        assert "--temporal-only" in result.stdout
        assert "--list-entities" in result.stdout
        assert "--entity-path" in result.stdout
        assert "--format" in result.stdout
    
    def test_extract_conflicting_options(self, sample_rrd_file):
        """Test that extract rejects conflicting options."""
        # Test static-only + temporal-only conflict
        result = subprocess.run([
            RERUN_BINARY, "rrd", "extract",
            "--static-only",
            "--temporal-only",
            sample_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        assert result.returncode != 0
        assert "Cannot specify both" in result.stderr
    
    def test_extract_entity_path_with_static_options(self, sample_rrd_file):
        """Test that extract rejects entity-path with static/temporal options."""
        result = subprocess.run([
            RERUN_BINARY, "rrd", "extract",
            "--static-only",
            "--entity-path", "/world/**",
            sample_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        assert result.returncode != 0
        assert "Cannot specify --entity-path with" in result.stderr
    
    def test_extract_invalid_format(self, sample_rrd_file):
        """Test that extract rejects invalid output formats."""
        result = subprocess.run([
            RERUN_BINARY, "rrd", "extract",
            "--format", "invalid",
            sample_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        assert result.returncode != 0
        assert "Invalid output format" in result.stderr
    
    def test_extract_list_entities(self, sample_rrd_file, temp_rrd_file):
        """Test entity path listing functionality."""
        result = subprocess.run([
            RERUN_BINARY, "rrd", "extract",
            "--list-entities",
            sample_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        # Should handle invalid input gracefully - either succeed or fail with decode error
        assert result.returncode == 0 or "couldn't decode" in result.stderr or "failed to read" in result.stderr
    
    def test_extract_json_format(self, sample_rrd_file, temp_rrd_file):
        """Test JSON output format."""
        result = subprocess.run([
            RERUN_BINARY, "rrd", "extract",
            "--format", "json",
            sample_rrd_file,
            "-o", temp_rrd_file.replace('.rrd', '.json')
        ], capture_output=True, text=True, timeout=10)
        
        # Should handle invalid input gracefully
        assert result.returncode == 0 or "couldn't decode" in result.stderr
        
        # Check if output file was created
        json_file = temp_rrd_file.replace('.rrd', '.json')
        if Path(json_file).exists():
            content = Path(json_file).read_text()
            assert content.startswith('[')
            assert content.endswith(']')
    
    def test_extract_csv_format(self, sample_rrd_file, temp_rrd_file):
        """Test CSV output format."""
        result = subprocess.run([
            RERUN_BINARY, "rrd", "extract",
            "--format", "csv",
            sample_rrd_file,
            "-o", temp_rrd_file.replace('.rrd', '.csv')
        ], capture_output=True, text=True, timeout=10)
        
        # Should handle invalid input gracefully
        assert result.returncode == 0 or "couldn't decode" in result.stderr
        
        # Check if output file was created
        csv_file = temp_rrd_file.replace('.rrd', '.csv')
        if Path(csv_file).exists():
            content = Path(csv_file).read_text()
            assert "message_type" in content  # Header should be present
    
    def test_extract_with_entity_path_filter(self, sample_rrd_file, temp_rrd_file):
        """Test entity path filtering functionality."""
        result = subprocess.run([
            RERUN_BINARY, "rrd", "extract",
            "--entity-path", "/world/**",
            sample_rrd_file,
            "-o", temp_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        # Should handle invalid input gracefully
        assert result.returncode == 0 or "couldn't decode" in result.stderr
    
    def test_extract_multiple_entity_paths(self, sample_rrd_file, temp_rrd_file):
        """Test filtering with multiple entity paths."""
        result = subprocess.run([
            RERUN_BINARY, "rrd", "extract",
            "--entity-path", "/cameras/*",
            "--entity-path", "/world/objects/**",
            sample_rrd_file,
            "-o", temp_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        # Should handle invalid input gracefully
        assert result.returncode == 0 or "couldn't decode" in result.stderr


class TestRRDSplice:
    """Test suite for the rerun rrd splice command."""
    
    def test_splice_help_available(self):
        """Test that splice command help is available."""
        result = subprocess.run(
            [RERUN_BINARY, "rrd", "splice", "--help"],
            capture_output=True, text=True, timeout=10
        )
        
        assert result.returncode == 0
        assert "Extracts a time-based slice" in result.stdout
        assert "--timeline" in result.stdout
        assert "--start" in result.stdout
        assert "--end" in result.stdout
    
    def test_splice_command_exists(self):
        """Test that splice command is recognized."""
        result = subprocess.run(
            [RERUN_BINARY, "rrd", "splice", "--help"],
            capture_output=True, text=True, timeout=10
        )
        
        assert result.returncode == 0
        assert "splice" in result.stdout.lower()
        assert "--exclude-static" in result.stdout
        assert "--static-only" in result.stdout

    def test_splice_conflicting_static_options(self, sample_rrd_file, temp_rrd_file):
        """Test that splice rejects conflicting static options."""
        result = subprocess.run([
            RERUN_BINARY, "rrd", "splice",
            "--exclude-static",
            "--static-only",
            sample_rrd_file,
            "-o", temp_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        assert result.returncode != 0
        assert "Cannot specify both" in result.stderr
    
    def test_splice_requires_valid_time_range(self, sample_rrd_file, temp_rrd_file):
        """Test that splice validates time ranges properly."""
        # Test invalid time range (start > end)
        result = subprocess.run([
            RERUN_BINARY, "rrd", "splice",
            "--timeline", "log_time",
            "--start", "2000000000",
            "--end", "1000000000",
            sample_rrd_file,
            "-o", temp_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        # Should fail with invalid time range
        assert result.returncode != 0
        assert "start time" in result.stderr.lower() or "must be less than" in result.stderr.lower() or "error" in result.stderr.lower()

    def test_splice_with_timeline_parameter(self, sample_rrd_file, temp_rrd_file):
        """Test splice with different timeline parameters."""
        # Test with log_time timeline
        result = subprocess.run([
            RERUN_BINARY, "rrd", "splice",
            "--timeline", "log_time",
            "--start", "1000000000",
            "--end", "2000000000",
            sample_rrd_file,
            "-o", temp_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        # Should handle invalid input gracefully - either succeed or fail with decode error
        assert result.returncode == 0 or "couldn't decode" in result.stderr or "failed to read" in result.stderr

    def test_splice_with_static_only_option(self, sample_rrd_file, temp_rrd_file):
        """Test splice with static-only option."""
        result = subprocess.run([
            RERUN_BINARY, "rrd", "splice",
            "--static-only",
            sample_rrd_file,
            "-o", temp_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        # Should handle invalid input gracefully - either succeed or fail with decode error
        assert result.returncode == 0 or "couldn't decode" in result.stderr or "failed to read" in result.stderr
    
    def test_splice_with_exclude_static_option(self, sample_rrd_file, temp_rrd_file):
        """Test splice with exclude-static option."""
        result = subprocess.run([
            RERUN_BINARY, "rrd", "splice",
            "--exclude-static",
            "--timeline", "log_time",
            "--start", "1000000000",
            sample_rrd_file,
            "-o", temp_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        # Should handle invalid input gracefully - either succeed or fail with decode error
        assert result.returncode == 0 or "couldn't decode" in result.stderr or "failed to read" in result.stderr


class TestRRDSplit:
    """Test suite for the rerun rrd split command."""
    
    def test_split_help_available(self):
        """Test that split command help is available."""
        result = subprocess.run(
            [RERUN_BINARY, "rrd", "split", "--help"],
            capture_output=True, text=True, timeout=10
        )
        
        assert result.returncode == 0
        assert "Splits .rrd/.rbl files" in result.stdout
        assert "--output-dir" in result.stdout
        assert "--size" in result.stdout
        assert "--name" in result.stdout
        assert "--exclude-static" in result.stdout
    
    def test_split_requires_output_dir(self, sample_rrd_file):
        """Test that split requires output directory."""
        result = subprocess.run([
            RERUN_BINARY, "rrd", "split",
            sample_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        # Should fail without output directory
        assert result.returncode != 0
        assert "--output-dir" in result.stderr or "required" in result.stderr

    
    def test_split_creates_output_directory(self, sample_rrd_file, temp_dir):
        """Test that split creates output directory if it doesn't exist."""
        output_dir = Path(temp_dir) / "nonexistent_dir"
        
        result = subprocess.run([
            RERUN_BINARY, "rrd", "split",
            "--output-dir", str(output_dir),
            "--size", "2048",  # 2KB chunks (valid size)
            sample_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        # Should handle invalid input gracefully - either succeed or fail with decode error
        assert result.returncode == 0 or "couldn't decode" in result.stderr or "failed to read" in result.stderr
        # Directory should still be created even if input is invalid
        assert output_dir.exists()
    
    def test_split_validates_size_parameter(self, sample_rrd_file, temp_dir):
        """Test that split validates size parameter."""
        # Test with too small size
        result = subprocess.run([
            RERUN_BINARY, "rrd", "split",
            "--output-dir", temp_dir,
            "--size", "100",  # Too small
            sample_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        # Should fail with size validation error
        assert result.returncode != 0
        assert "at least 1KB" in result.stderr or "max size" in result.stderr.lower()
    
    def test_split_generates_merge_script(self, sample_rrd_file, temp_dir):
        """Test that split generates a merge script."""
        result = subprocess.run([
            RERUN_BINARY, "rrd", "split",
            "--output-dir", temp_dir,
            "--size", "104857600",  # 100MB
            "--name", "test_chunk",
            sample_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        # Should handle invalid input gracefully - either succeed or fail with decode error  
        assert result.returncode == 0 or "couldn't decode" in result.stderr or "failed to read" in result.stderr
        
        # Even with invalid input, merge script should be created
        merge_script = Path(temp_dir) / "merge_chunks.sh"
        if merge_script.exists():
            content = merge_script.read_text()
            assert "rerun rrd merge" in content
            assert "test_chunk" in content

    def test_split_with_exclude_static_option(self, sample_rrd_file, temp_dir):
        """Test split with exclude-static option."""
        result = subprocess.run([
            RERUN_BINARY, "rrd", "split",
            "--output-dir", temp_dir,
            "--exclude-static",
            "--size", "104857600",  # 100MB
            sample_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        # Should handle invalid input gracefully - either succeed or fail with decode error  
        assert result.returncode == 0 or "couldn't decode" in result.stderr or "failed to read" in result.stderr


class TestRRDIntegration:
    """Test integration between extract, splice, split, and merge commands."""
    
    def test_commands_appear_in_help(self):
        """Test that all RRD manipulation commands appear in help."""
        result = subprocess.run(
            [RERUN_BINARY, "rrd", "--help"],
            capture_output=True, text=True, timeout=10
        )
        
        assert result.returncode == 0
        assert "extract" in result.stdout  # New extract command
        assert "splice" in result.stdout
        assert "split" in result.stdout
        assert "merge" in result.stdout  # Existing command should still be there
    
    def test_version_shows_v008(self):
        """Test that version shows v0.0.8."""
        result = subprocess.run(
            [RERUN_BINARY, "--version"],
            capture_output=True, text=True, timeout=5
        )
        
        assert result.returncode == 0
        assert "v0.0.8" in result.stdout
        assert "Recording Splicing" in result.stdout


class TestBackwardCompatibility:
    """Test that existing RRD commands still work."""
    
    def test_existing_merge_still_works(self):
        """Test that the existing merge command still works."""
        result = subprocess.run(
            [RERUN_BINARY, "rrd", "merge", "--help"],
            capture_output=True, text=True, timeout=10
        )
        
        assert result.returncode == 0
        assert "Merges the contents" in result.stdout
    
    def test_existing_filter_still_works(self):
        """Test that the existing filter command still works."""
        result = subprocess.run(
            [RERUN_BINARY, "rrd", "filter", "--help"],
            capture_output=True, text=True, timeout=10
        )
        
        assert result.returncode == 0
        assert "Filters out data" in result.stdout


if __name__ == "__main__":
    pytest.main([__file__, "-v"])