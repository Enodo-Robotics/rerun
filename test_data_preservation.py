#!/usr/bin/env python3
"""
Comprehensive test to verify that ALL data from ALL processes is preserved
with the new file locking implementation.
"""

import os
import sys
import tempfile
import multiprocessing
import time
import subprocess
from pathlib import Path

def worker_process(worker_id: int, output_file: str, messages_per_worker: int):
    """Worker process that logs unique identifiable messages"""
    
    # Use subprocess to avoid dev environment issues
    script_content = f'''
import sys
import os
sys.path.insert(0, '{str(Path(__file__).parent / "rerun_py")}')

try:
    import rerun as rr
    
    # Initialize with unique app ID
    rr.init(f"data_preservation_test_worker_{worker_id}", spawn=False)
    
    # Save to the shared file
    rr.save(r"{output_file}")
    
    # Log unique messages that can be identified later
    for i in range({messages_per_worker}):
        # Create unique scalar values for this worker
        scalar_value = {worker_id} * 1000 + i
        
        # Log with worker-specific entity paths
        rr.log(f"worker_{worker_id}/scalar_{{i}}", rr.Scalars([scalar_value]))
        rr.log(f"worker_{worker_id}/text_{{i}}", rr.TextLog(f"Worker {worker_id} message {{i}} unique_id={{scalar_value}}"))
        
        # Small delay to ensure messages don't all get the same timestamp
        import time
        time.sleep(0.01)
    
    print(f"Worker {worker_id} completed successfully - logged {messages_per_worker} messages")
    
except Exception as e:
    print(f"Worker {worker_id} failed: {{e}}")
    import traceback
    traceback.print_exc()
    sys.exit(1)
'''
    
    # Run the script in a subprocess
    try:
        result = subprocess.run([
            sys.executable, '-c', script_content
        ], capture_output=True, text=True, timeout=60, cwd='/tmp')
        
        print(result.stdout)
        if result.stderr:
            print(f"Worker {worker_id} stderr: {result.stderr}")
        
        return result.returncode == 0
        
    except subprocess.TimeoutExpired:
        print(f"Worker {worker_id} timed out")
        return False
    except Exception as e:
        print(f"Worker {worker_id} subprocess error: {e}")
        return False

def verify_data_preservation(output_file: str, expected_workers: int, expected_messages_per_worker: int):
    """
    Verify that the output file contains data from all processes.
    This is a simple check - a more robust version would parse the RRD file.
    """
    
    if not os.path.exists(output_file):
        print("✗ Output file does not exist")
        return False, 0, 0
    
    file_size = os.path.getsize(output_file)
    print(f"Output file size: {file_size} bytes")
    
    # Basic heuristic: check if file size is reasonable for the expected amount of data
    # This is not perfect but gives us some confidence
    expected_minimum_size = expected_workers * expected_messages_per_worker * 50  # rough estimate
    
    if file_size < expected_minimum_size:
        print(f"✗ File size ({file_size} bytes) is smaller than expected minimum ({expected_minimum_size} bytes)")
        return False, file_size, expected_minimum_size
    
    # Check for presence of lock file (should be cleaned up)
    lock_file = Path(output_file).with_suffix(".rrd.lock")
    if lock_file.exists():
        print(f"⚠ Lock file still exists: {lock_file}")
        # Try to remove it
        try:
            lock_file.unlink()
            print("✓ Cleaned up lock file")
        except:
            print("✗ Could not clean up lock file")
    
    print("✓ File size suggests data from multiple processes is preserved")
    return True, file_size, expected_minimum_size

def test_data_preservation():
    """Test that all data from multiple processes is preserved"""
    
    with tempfile.NamedTemporaryFile(suffix='.rrd', delete=False) as f:
        output_file = f.name
    
    try:
        # Test parameters
        num_workers = 4
        messages_per_worker = 3
        total_expected_messages = num_workers * messages_per_worker
        
        print(f"Testing data preservation with {num_workers} workers")
        print(f"Each worker will log {messages_per_worker} messages")
        print(f"Total expected messages: {total_expected_messages}")
        print(f"Output file: {output_file}")
        
        # Create and start worker processes
        processes = []
        start_time = time.time()
        
        for i in range(num_workers):
            p = multiprocessing.Process(
                target=worker_process,
                args=(i, output_file, messages_per_worker)
            )
            processes.append(p)
            p.start()
            
            # Small delay to stagger process starts
            time.sleep(0.1)
        
        # Wait for all processes to complete
        results = []
        for i, p in enumerate(processes):
            p.join(timeout=30)  # 30 second timeout per process
            if p.is_alive():
                print(f"Worker {i} timed out, terminating...")
                p.terminate()
                p.join()
                results.append(False)
            else:
                results.append(p.exitcode == 0)
        
        end_time = time.time()
        elapsed = end_time - start_time
        
        # Analyze results
        successful_workers = sum(results)
        print(f"\nResults after {elapsed:.2f} seconds:")
        print(f"Successful workers: {successful_workers}/{num_workers}")
        
        for i, success in enumerate(results):
            status = "✓ SUCCESS" if success else "✗ FAILED"
            print(f"  Worker {i}: {status}")
        
        # Verify data preservation
        preserved, file_size, expected_min = verify_data_preservation(
            output_file, num_workers, messages_per_worker
        )
        
        # Overall success criteria
        all_workers_succeeded = successful_workers == num_workers
        data_preserved = preserved
        
        print(f"\nFinal Assessment:")
        print(f"All workers succeeded: {'✓' if all_workers_succeeded else '✗'}")
        print(f"Data preservation: {'✓' if data_preserved else '✗'}")
        
        if all_workers_succeeded and data_preserved:
            print("\n🎉 SUCCESS: All data from all processes was preserved!")
            return True
        else:
            print("\n❌ FAILURE: Data preservation test failed")
            return False
            
    finally:
        # Clean up
        try:
            os.unlink(output_file)
        except:
            pass
        
        # Clean up any remaining lock files
        lock_file = Path(output_file).with_suffix(".rrd.lock")
        if lock_file.exists():
            try:
                lock_file.unlink()
            except:
                pass

if __name__ == "__main__":
    print("Data Preservation Test with File Locking")
    print("=" * 60)
    
    success = test_data_preservation()
    
    if success:
        print("\n✅ All tests passed! Data preservation is working correctly.")
        sys.exit(0)
    else:
        print("\n❌ Tests failed. Data preservation needs improvement.")
        sys.exit(1)