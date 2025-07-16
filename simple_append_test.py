#!/usr/bin/env python3
"""
Simple test to verify that the append functionality works correctly
"""

import sys
import os
import tempfile
from pathlib import Path

# Add rerun_py to path
sys.path.insert(0, str(Path(__file__).parent / "rerun_py"))

try:
    import rerun as rr
    print("✓ Successfully imported rerun")
except ImportError as e:
    print(f"✗ Failed to import rerun: {e}")
    sys.exit(1)

def test_append_functionality():
    """Test that multiple saves to the same file work correctly"""
    
    with tempfile.NamedTemporaryFile(suffix='.rrd', delete=False) as f:
        output_file = f.name
    
    try:
        print(f"Testing append functionality with file: {output_file}")
        
        # First save
        print("Creating first recording...")
        rr.init("test_app_1", spawn=False)
        rr.save(output_file)
        
        rr.log("data/counter", rr.Scalar(1))
        rr.log("data/message", rr.TextLog("First message"))
        
        # Check file size after first save
        if os.path.exists(output_file):
            size1 = os.path.getsize(output_file)
            print(f"File size after first save: {size1} bytes")
        else:
            print("✗ File not created after first save")
            return False
        
        # Second save (should append)
        print("Creating second recording...")
        rr.init("test_app_2", spawn=False)
        rr.save(output_file)
        
        rr.log("data/counter", rr.Scalar(2))
        rr.log("data/message", rr.TextLog("Second message"))
        
        # Check file size after second save
        if os.path.exists(output_file):
            size2 = os.path.getsize(output_file)
            print(f"File size after second save: {size2} bytes")
            
            if size2 > size1:
                print("✓ File grew, indicating append behavior")
                return True
            else:
                print("✗ File did not grow, data may have been overwritten")
                return False
        else:
            print("✗ File not found after second save")
            return False
            
    finally:
        try:
            os.unlink(output_file)
        except:
            pass

if __name__ == "__main__":
    print("Simple Append Test")
    print("=" * 30)
    
    success = test_append_functionality()
    
    if success:
        print("\n🎉 Append functionality works correctly!")
        sys.exit(0)
    else:
        print("\n❌ Append functionality failed!")
        sys.exit(1)