#!/usr/bin/env python3
"""
Test the installed wheel (not development environment)
"""

import os
import sys
import tempfile
import subprocess

def test_with_installed_wheel():
    """Test using the installed wheel by running in subprocess"""
    
    with tempfile.NamedTemporaryFile(suffix='.rrd', delete=False) as f:
        output_file = f.name
    
    try:
        print(f"Testing with installed wheel using file: {output_file}")
        
        # First save
        first_script = f'''
import rerun as rr
rr.init("test_app_1", spawn=False)
rr.save(r"{output_file}")
rr.log("data/counter", rr.Scalars([1]))
rr.log("data/message", rr.TextLog("First message"))
print("First save completed")
'''
        
        # Run first script
        result1 = subprocess.run([
            sys.executable, '-c', first_script
        ], capture_output=True, text=True, cwd='/tmp')
        
        if result1.returncode != 0:
            print(f"First save failed: {result1.stderr}")
            return False
            
        print("First save output:", result1.stdout.strip())
        
        # Check file size after first save
        if os.path.exists(output_file):
            size1 = os.path.getsize(output_file)
            print(f"File size after first save: {size1} bytes")
        else:
            print("✗ File not created after first save")
            return False
        
        # Second save
        second_script = f'''
import rerun as rr
rr.init("test_app_2", spawn=False)
rr.save(r"{output_file}")
rr.log("data/counter", rr.Scalars([2]))
rr.log("data/message", rr.TextLog("Second message"))
print("Second save completed")
'''
        
        # Run second script
        result2 = subprocess.run([
            sys.executable, '-c', second_script
        ], capture_output=True, text=True, cwd='/tmp')
        
        if result2.returncode != 0:
            print(f"Second save failed: {result2.stderr}")
            return False
            
        print("Second save output:", result2.stdout.strip())
        
        # Check file size after second save
        if os.path.exists(output_file):
            size2 = os.path.getsize(output_file)
            print(f"File size after second save: {size2} bytes")
            
            if size2 > size1:
                print("✓ File grew, indicating data was appended")
                return True
            elif size2 == size1:
                print("? File size stayed the same, may be due to compression")
                return False
            else:
                print("✗ File size decreased, data was likely overwritten")
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
    print("Testing Installed Wheel Append Functionality")
    print("=" * 50)
    
    success = test_with_installed_wheel()
    
    if success:
        print("\n🎉 Append functionality works correctly!")
        sys.exit(0)
    else:
        print("\n❌ Append functionality failed!")
        sys.exit(1)