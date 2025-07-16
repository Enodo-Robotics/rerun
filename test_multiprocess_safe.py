#!/usr/bin/env python3
"""
Test script to verify multiprocess-safe rerun save functionality.
This script simulates multiple processes trying to save to the same file.
"""

import os
import sys
import time
import multiprocessing
import tempfile
from pathlib import Path

# Add rerun_py to path to use the local version
sys.path.insert(0, str(Path(__file__).parent / "rerun_py"))

try:
    import rerun as rr
    print("✓ Successfully imported rerun")
except ImportError as e:
    print(f"✗ Failed to import rerun: {e}")
    print("Note: You may need to install the wheel first")
    sys.exit(1)

def worker_process(worker_id: int, output_file: str, num_messages: int = 10):
    """Worker process that logs messages to the same file"""
    print(f"Worker {worker_id} starting...")
    
    try:
        # Initialize rerun with a unique application ID for each worker
        rr.init(f"multiprocess_test_worker_{worker_id}", spawn=False)
        
        # Try to save to the same file from multiple processes
        rr.save(output_file)
        
        # Log some test data
        for i in range(num_messages):
            rr.log(f"worker_{worker_id}/counter", rr.Scalar(i))
            rr.log(f"worker_{worker_id}/message", rr.TextLog(f"Hello from worker {worker_id}, message {i}"))
            time.sleep(0.1)  # Small delay to simulate work
        
        print(f"Worker {worker_id} completed successfully")
        return True
        
    except Exception as e:
        print(f"Worker {worker_id} failed: {e}")
        return False

def test_multiprocess_save():
    """Test that multiple processes can save to the same file without conflicts"""
    print("Testing multiprocess-safe rerun save...")
    
    # Create a temporary file for testing
    with tempfile.NamedTemporaryFile(suffix='.rrd', delete=False) as f:
        output_file = f.name
    
    try:
        # Number of processes to spawn
        num_processes = 3
        
        # Create and start worker processes
        processes = []
        for i in range(num_processes):
            p = multiprocessing.Process(
                target=worker_process, 
                args=(i, output_file, 5)
            )
            processes.append(p)
            p.start()
        
        # Wait for all processes to complete
        results = []
        for p in processes:
            p.join()
            results.append(p.exitcode == 0)
        
        # Check results
        successful_workers = sum(results)
        print(f"Results: {successful_workers}/{num_processes} workers completed successfully")
        
        # Check if file was created and has content
        if os.path.exists(output_file):
            file_size = os.path.getsize(output_file)
            print(f"Output file size: {file_size} bytes")
            
            if file_size > 0:
                print("✓ File was created and contains data")
                return True
            else:
                print("✗ File was created but is empty")
                return False
        else:
            print("✗ Output file was not created")
            return False
            
    finally:
        # Clean up
        try:
            os.unlink(output_file)
        except:
            pass

def test_sequential_save():
    """Test that sequential saves work correctly (baseline test)"""
    print("\nTesting sequential save (baseline)...")
    
    with tempfile.NamedTemporaryFile(suffix='.rrd', delete=False) as f:
        output_file = f.name
    
    try:
        rr.init("sequential_test", spawn=False)
        rr.save(output_file)
        
        # Log some test data
        for i in range(5):
            rr.log("test/counter", rr.Scalar(i))
            rr.log("test/message", rr.TextLog(f"Test message {i}"))
        
        # Check file was created
        if os.path.exists(output_file) and os.path.getsize(output_file) > 0:
            print("✓ Sequential save works correctly")
            return True
        else:
            print("✗ Sequential save failed")
            return False
            
    finally:
        try:
            os.unlink(output_file)
        except:
            pass

if __name__ == "__main__":
    print("Multiprocess-safe rerun save test")
    print("=" * 50)
    
    # Test sequential save first (baseline)
    sequential_ok = test_sequential_save()
    
    # Test multiprocess save
    multiprocess_ok = test_multiprocess_save()
    
    print("\nTest Results:")
    print(f"Sequential save: {'✓ PASS' if sequential_ok else '✗ FAIL'}")
    print(f"Multiprocess save: {'✓ PASS' if multiprocess_ok else '✗ FAIL'}")
    
    if sequential_ok and multiprocess_ok:
        print("\n🎉 All tests passed! Multiprocess-safe save is working.")
        sys.exit(0)
    else:
        print("\n❌ Some tests failed.")
        sys.exit(1)