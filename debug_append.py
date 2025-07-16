#!/usr/bin/env python3
"""
Debug script to understand what's happening with append functionality
"""

import os
import sys
import tempfile
import subprocess
from pathlib import Path

# Add rerun_py to path to use the development version
sys.path.insert(0, str(Path(__file__).parent / "rerun_py"))

import rerun as rr

def debug_file_behavior():
    """Debug what happens when we save multiple times"""
    
    with tempfile.NamedTemporaryFile(suffix='.rrd', delete=False) as f:
        output_file = f.name
    
    try:
        print(f"Testing with file: {output_file}")
        print(f"Using rerun from: {rr.__file__}")
        
        # First save
        print("\n--- First Save ---")
        rr.init("debug_app_1", spawn=False)
        rr.save(output_file)
        print(f"File exists after init: {os.path.exists(output_file)}")
        if os.path.exists(output_file):
            print(f"File size after init: {os.path.getsize(output_file)} bytes")
        
        rr.log("data/counter", rr.Scalars([1]))
        rr.log("data/message", rr.TextLog("First message"))
        
        # Force flush
        print("Flushing...")
        
        if os.path.exists(output_file):
            size1 = os.path.getsize(output_file)
            print(f"File size after first save: {size1} bytes")
            
            # Read some bytes to see what's in the file
            with open(output_file, 'rb') as f:
                first_bytes = f.read(100)
                print(f"First 100 bytes: {first_bytes[:50]}...")
        else:
            print("✗ File not created after first save")
            return False
        
        # Second save
        print("\n--- Second Save ---")
        rr.init("debug_app_2", spawn=False)
        rr.save(output_file)
        print(f"File exists after second init: {os.path.exists(output_file)}")
        if os.path.exists(output_file):
            print(f"File size after second init: {os.path.getsize(output_file)} bytes")
        
        rr.log("data/counter", rr.Scalars([2]))
        rr.log("data/message", rr.TextLog("Second message"))
        
        # Force flush
        print("Flushing...")
        
        if os.path.exists(output_file):
            size2 = os.path.getsize(output_file)
            print(f"File size after second save: {size2} bytes")
            
            # Read some bytes to see what's in the file
            with open(output_file, 'rb') as f:
                second_bytes = f.read(100)
                print(f"First 100 bytes: {second_bytes[:50]}...")
            
            print(f"Size change: {size2 - size1} bytes")
            print(f"Files are same: {first_bytes == second_bytes}")
        else:
            print("✗ File not found after second save")
            return False
            
    finally:
        try:
            os.unlink(output_file)
        except:
            pass

if __name__ == "__main__":
    debug_file_behavior()