#!/usr/bin/env python3
"""Integration tests for Rerun v0.0.7 continuous saving functionality."""

import pytest
import rerun as rr
import numpy as np
import multiprocessing
import subprocess
import time
from pathlib import Path
import tempfile
import os


@pytest.fixture
def temp_recording_file():
    """Create a temporary recording file for testing."""
    with tempfile.NamedTemporaryFile(delete=False, suffix='.rrd') as tmp:
        yield tmp.name
    # Cleanup
    if Path(tmp.name).exists():
        Path(tmp.name).unlink()


def client_process(client_id: int, duration: int = 5, port: int = 9876):
    """Client process that sends data to the gRPC server."""
    try:
        rr.init(f"test_client_{client_id}", spawn=False)
        rr.connect_grpc(f"rerun+http://127.0.0.1:{port}/proxy")
        
        start_time = time.time()
        frame = 0
        
        while time.time() - start_time < duration:
            rr.set_time("frame", sequence=frame)
            
            if client_id == 1:
                points = np.random.rand(10, 3) * 5
                rr.log(f"client_{client_id}/points", rr.Points3D(points))
            elif client_id == 2:
                image = np.random.randint(0, 255, size=(16, 16, 3), dtype=np.uint8)
                rr.log(f"client_{client_id}/image", rr.Image(image))
            elif client_id == 3:
                value = np.sin(frame * 0.2) * 100
                rr.log(f"client_{client_id}/scalar", rr.Scalars([value]))
            
            rr.log(f"client_{client_id}/status", rr.TextLog(f"Frame {frame}"))
            
            frame += 1
            time.sleep(0.05)  # Faster for tests
        
        rr.disconnect()
        return frame
    except Exception as e:
        print(f"Client {client_id} error: {e}")
        return 0


class TestContinuousSave:
    """Test suite for continuous saving functionality introduced in v0.0.7."""
    
    @pytest.fixture(autouse=True)
    def cleanup_processes(self):
        """Kill any existing rerun processes before and after tests."""
        subprocess.run(["killall", "rerun"], capture_output=True)
        time.sleep(0.5)
        yield
        subprocess.run(["killall", "rerun"], capture_output=True)
        
    def test_grpc_server_with_continuous_save(self, temp_recording_file):
        """Test gRPC server with continuous saving at intervals."""
        # Start server with continuous saving
        server_process = subprocess.Popen([
            "rerun",
            "--serve-grpc", 
            "--port", "9876",
            "--save", temp_recording_file,
            "--save-interval", "2"  # Save every 2 seconds
        ])
        
        time.sleep(1)  # Let server start
        
        try:
            # Start multiple clients
            clients = []
            for i in range(1, 4):
                p = multiprocessing.Process(target=client_process, args=(i, 5))  # 5 seconds
                p.start()
                clients.append(p)
                time.sleep(0.1)
            
            # Wait for all clients to finish
            for p in clients:
                p.join(timeout=10)
                if p.is_alive():
                    p.terminate()
            
            # Wait for final save
            time.sleep(3)
        finally:
            server_process.terminate()
            server_process.wait(timeout=5)
        
        # Verify file was created and has content
        assert Path(temp_recording_file).exists(), "Recording file should exist"
        assert Path(temp_recording_file).stat().st_size > 1000, "Recording file should have substantial content"
        
    def test_recording_integrity_and_dataframe_api(self, temp_recording_file):
        """Test that continuous saving produces valid recordings compatible with dataframe API."""
        # First create a recording
        self.test_grpc_server_with_continuous_save(temp_recording_file)
        
        # Now verify it can be loaded and queried
        archive = rr.dataframe.load_archive(temp_recording_file)
        recordings = archive.all_recordings()
        
        assert len(recordings) > 0, "Should have at least one recording"
        
        recording = recordings[0]
        schema = recording.schema()
        timelines = schema.index_columns()
        components = schema.component_columns()
        
        assert len(timelines) > 0, "Should have timelines"
        assert len(components) > 0, "Should have component columns"
        
        # Test extracting different data types
        data_extracted = 0
        
        # Test scalar data extraction
        try:
            view = recording.view(index="frame", contents="client_3/scalar")
            table = view.select().read_all()
            if len(table) > 0:
                df = table.to_pandas()
                scalar_cols = [col for col in df.columns if 'scalar' in col.lower()]
                if scalar_cols:
                    data_extracted += 1
        except Exception:
            pass
        
        # Test point cloud data extraction
        try:
            view = recording.view(index="frame", contents="client_1/points")
            table = view.select().read_all()
            if len(table) > 0:
                data_extracted += 1
        except Exception:
            pass
        
        # Test image data extraction
        try:
            view = recording.view(index="frame", contents="client_2/image")
            table = view.select().read_all()
            if len(table) > 0:
                data_extracted += 1
        except Exception:
            pass
        
        assert data_extracted >= 2, f"Should extract at least 2 data types, got {data_extracted}"
        
    def test_file_rotation(self, temp_recording_file):
        """Test file rotation feature with --rotate-files flag."""
        base_path = Path(temp_recording_file)
        base_dir = base_path.parent
        base_name = base_path.stem
        
        # Start server with file rotation
        server_process = subprocess.Popen([
            "rerun",
            "--serve-grpc", 
            "--port", "9877",
            "--save", temp_recording_file,
            "--save-interval", "2",
            "--rotate-files"
        ])
        
        time.sleep(1)
        
        try:
            # Send some data
            p = multiprocessing.Process(target=client_process, args=(1, 4, 9877))
            p.start()
            p.join(timeout=10)
            if p.is_alive():
                p.terminate()
                
            # Wait for rotation
            time.sleep(3)
        finally:
            server_process.terminate()
            server_process.wait(timeout=5)
        
        # Check if timestamped files were created
        timestamped_files = list(base_dir.glob(f"{base_name}_ts*.rrd"))
        assert len(timestamped_files) > 0, "Should create timestamped files with rotation"
        
        # Cleanup timestamped files
        for f in timestamped_files:
            f.unlink()


class TestBackwardCompatibility:
    """Test backward compatibility with existing functionality."""
    
    def test_regular_grpc_save_still_works(self, temp_recording_file):
        """Test that regular gRPC saving (without intervals) still works."""
        server_process = subprocess.Popen([
            "rerun",
            "--serve-grpc", 
            "--port", "9878",
            "--save", temp_recording_file
        ])
        
        time.sleep(1)
        
        try:
            # Send some data
            p = multiprocessing.Process(target=client_process, args=(1, 3, 9878))
            p.start()
            p.join(timeout=10)
            if p.is_alive():
                p.terminate()
                
            time.sleep(1)
        finally:
            server_process.terminate()
            server_process.wait(timeout=5)
        
        # Verify file was created
        assert Path(temp_recording_file).exists(), "Recording file should exist"
        assert Path(temp_recording_file).stat().st_size > 100, "Recording file should have content"


if __name__ == "__main__":
    pytest.main([__file__, "-v"])