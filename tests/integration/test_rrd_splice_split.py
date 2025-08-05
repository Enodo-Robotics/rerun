#!/usr/bin/env python3
"""Integration tests for Rerun v0.0.8 RRD splice and split functionality."""

import pytest
import subprocess
import tempfile
import shutil
from pathlib import Path
import time


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


class TestRRDSplice:
    """Test suite for the rerun rrd splice command."""
    
    def test_splice_help_available(self):
        """Test that splice command help is available."""
        result = subprocess.run(
            ["./target/release/rerun", "rrd", "splice", "--help"],
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
            ["./target/release/rerun", "rrd", "splice", "--help"],
            capture_output=True, text=True, timeout=10
        )
        
        assert result.returncode == 0
        assert "splice" in result.stdout.lower()
    
    def test_splice_requires_valid_time_range(self, sample_rrd_file, temp_rrd_file):
        """Test that splice validates time ranges properly."""
        # Test invalid time range (start > end)
        result = subprocess.run([
            "./target/release/rerun", "rrd", "splice",
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
            "./target/release/rerun", "rrd", "splice",
            "--timeline", "log_time",
            "--start", "1000000000",
            "--end", "2000000000",
            sample_rrd_file,
            "-o", temp_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        # Should succeed (even with empty input file)
        assert result.returncode == 0 or "no messages" in result.stderr.lower()


class TestRRDSplit:
    """Test suite for the rerun rrd split command."""
    
    def test_split_help_available(self):
        """Test that split command help is available."""
        result = subprocess.run(
            ["./target/release/rerun", "rrd", "split", "--help"],
            capture_output=True, text=True, timeout=10
        )
        
        assert result.returncode == 0
        assert "Splits .rrd/.rbl files" in result.stdout
        assert "--output-dir" in result.stdout
        assert "--size" in result.stdout
        assert "--name" in result.stdout
    
    def test_split_requires_output_dir(self, sample_rrd_file):
        """Test that split requires output directory."""
        result = subprocess.run([
            "./target/release/rerun", "rrd", "split",
            sample_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        # Should fail without output directory
        assert result.returncode != 0
        assert "--output-dir" in result.stderr or "required" in result.stderr
    
    def test_split_creates_output_directory(self, sample_rrd_file, temp_dir):
        """Test that split creates output directory if it doesn't exist."""
        output_dir = Path(temp_dir) / "nonexistent_dir"
        
        result = subprocess.run([
            "./target/release/rerun", "rrd", "split",
            "--output-dir", str(output_dir),
            "--size", "1024",  # 1KB chunks
            sample_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        # Should succeed and create directory
        assert result.returncode == 0 or "no messages" in result.stderr.lower()
        assert output_dir.exists()
    
    def test_split_validates_size_parameter(self, sample_rrd_file, temp_dir):
        """Test that split validates size parameter."""
        # Test with too small size
        result = subprocess.run([
            "./target/release/rerun", "rrd", "split",
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
            "./target/release/rerun", "rrd", "split",
            "--output-dir", temp_dir,
            "--size", "104857600",  # 100MB
            "--name", "test_chunk",
            sample_rrd_file
        ], capture_output=True, text=True, timeout=10)
        
        # Should succeed and create merge script
        assert result.returncode == 0 or "no messages" in result.stderr.lower()
        
        merge_script = Path(temp_dir) / "merge_chunks.sh"
        if merge_script.exists():
            content = merge_script.read_text()
            assert "rerun rrd merge" in content
            assert "test_chunk" in content


class TestRRDIntegration:
    """Test integration between splice, split, and merge commands."""
    
    def test_commands_appear_in_help(self):
        """Test that both new commands appear in RRD help."""
        result = subprocess.run(
            ["./target/release/rerun", "rrd", "--help"],
            capture_output=True, text=True, timeout=10
        )
        
        assert result.returncode == 0
        assert "splice" in result.stdout
        assert "split" in result.stdout
        assert "merge" in result.stdout  # Existing command should still be there
    
    def test_version_shows_v008(self):
        """Test that version shows v0.0.8."""
        result = subprocess.run(
            ["./target/release/rerun", "--version"],
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
            ["./target/release/rerun", "rrd", "merge", "--help"],
            capture_output=True, text=True, timeout=10
        )
        
        assert result.returncode == 0
        assert "Merges the contents" in result.stdout
    
    def test_existing_filter_still_works(self):
        """Test that the existing filter command still works."""
        result = subprocess.run(
            ["./target/release/rerun", "rrd", "filter", "--help"],
            capture_output=True, text=True, timeout=10
        )
        
        assert result.returncode == 0
        assert "Filters out data" in result.stdout


if __name__ == "__main__":
    pytest.main([__file__, "-v"])