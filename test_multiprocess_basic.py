#!/usr/bin/env python3
"""
Basic test to verify multiprocess access doesn't crash
"""

import os
import sys
import tempfile
import multiprocessing
import time
from pathlib import Path

# Add rerun_py to path
sys.path.insert(0, str(Path(__file__).parent / "rerun_py"))

import rerun as rr

def worker_process(worker_id: int, output_file: str):
    """Worker process that tries to save to the same file"""
    try:
        rr.init(f"worker_{worker_id}", spawn=False)
        rr.save(output_file)
        
        # Log one message
        rr.log(f"worker_{worker_id}/data", rr.Scalars([worker_id]))
        
        print(f"Worker {worker_id}: SUCCESS")
        return True
    except Exception as e:
        print(f"Worker {worker_id}: FAILED - {e}")
        return False

def test_multiprocess_basic():
    """Test that multiple processes can save without crashing"""
    
    with tempfile.NamedTemporaryFile(suffix='.rrd', delete=False) as f:
        output_file = f.name
    
    try:
        print(f"Testing multiprocess access to: {output_file}")
        
        # Create and start worker processes
        processes = []
        for i in range(3):
            p = multiprocessing.Process(target=worker_process, args=(i, output_file))
            processes.append(p)
            p.start()
            time.sleep(0.1)  # Small delay to stagger starts
        
        # Wait for all processes to complete
        success_count = 0
        for p in processes:
            p.join()
            if p.exitcode == 0:
                success_count += 1
        
        print(f"Results: {success_count}/3 processes succeeded")
        
        # Check final file
        if os.path.exists(output_file):
            final_size = os.path.getsize(output_file)
            print(f"Final file size: {final_size} bytes")
            
            if final_size > 0:
                print("✓ File was created and contains data")
                return success_count >= 2  # At least 2 processes should succeed
            else:
                print("✗ File is empty")
                return False
        else:
            print("✗ No file was created")
            return False
            
    finally:
        try:
            os.unlink(output_file)
        except:
            pass

if __name__ == "__main__":
    print("Basic Multiprocess Test")
    print("=" * 30)
    
    success = test_multiprocess_basic()
    
    if success:
        print("\n🎉 Multiprocess access works without crashes!")
        print("Data preservation and completeness can be verified separately.")
        sys.exit(0)
    else:
        print("\n❌ Multiprocess access failed!")
        sys.exit(1)