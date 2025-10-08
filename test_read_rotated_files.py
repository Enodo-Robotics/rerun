#!/usr/bin/env python3
"""
Simple test to verify that rotated RRD files can be read using the Python API.

This script reads RRD files (both rotated and non-rotated) and verifies
they can be queried correctly.
"""

import rerun as rr
import sys
from pathlib import Path


def read_and_analyze_rrd(rrd_path: Path) -> dict:
    """Read an RRD file and return statistics about its content."""
    print(f"\nAnalyzing: {rrd_path.name}")
    print(f"  Size: {rrd_path.stat().st_size:,} bytes")

    try:
        # Load the recording
        recording = rr.dataframe.load_recording(str(rrd_path))

        # Query all data
        view = recording.view(index="frame", contents="/**")

        entity_paths = set()
        total_rows = 0

        # Iterate through batches
        for batch in view.select():
            # Get entity path from the batch
            if hasattr(batch, 'entity_path'):
                entity_paths.add(batch.entity_path())

            total_rows += len(batch)

        stats = {
            "file": rrd_path.name,
            "size_bytes": rrd_path.stat().st_size,
            "entities": len(entity_paths),
            "total_rows": total_rows,
            "success": True,
        }

        print(f"  ✓ Successfully read")
        print(f"  Unique entity paths: {len(entity_paths)}")
        print(f"  Total rows: {total_rows}")

        return stats

    except Exception as e:
        print(f"  ✗ Error reading file: {e}")
        return {
            "file": rrd_path.name,
            "size_bytes": rrd_path.stat().st_size,
            "entities": 0,
            "total_rows": 0,
            "success": False,
            "error": str(e),
        }


def main():
    if len(sys.argv) < 2:
        print("Usage: python test_read_rotated_files.py <rrd_file_or_directory> [<rrd_file2> ...]")
        print("\nExamples:")
        print("  python test_read_rotated_files.py test.rrd")
        print("  python test_read_rotated_files.py rotated_dir/")
        print("  python test_read_rotated_files.py file1.rrd file2.rrd file3.rrd")
        return 1

    print("=" * 80)
    print("Testing RRD File Reading with Python API")
    print("=" * 80)

    # Collect all RRD files from arguments
    rrd_files = []
    for arg in sys.argv[1:]:
        path = Path(arg)
        if path.is_file() and path.suffix == ".rrd":
            rrd_files.append(path)
        elif path.is_dir():
            rrd_files.extend(sorted(path.glob("*.rrd")))

    if not rrd_files:
        print("No RRD files found!")
        return 1

    print(f"\nFound {len(rrd_files)} RRD file(s) to analyze")

    # Analyze each file
    results = []
    for rrd_file in rrd_files:
        stats = read_and_analyze_rrd(rrd_file)
        results.append(stats)

    # Summary
    print("\n" + "=" * 80)
    print("SUMMARY")
    print("=" * 80)

    successful = [r for r in results if r["success"]]
    failed = [r for r in results if not r["success"]]

    print(f"\nTotal files: {len(results)}")
    print(f"Successfully read: {len(successful)}")
    print(f"Failed: {len(failed)}")

    if successful:
        total_size = sum(r["size_bytes"] for r in successful)
        total_entities = sum(r["entities"] for r in successful)
        total_rows = sum(r["total_rows"] for r in successful)

        print(f"\nAcross all successful files:")
        print(f"  Total size: {total_size:,} bytes")
        print(f"  Total unique entities: {total_entities}")
        print(f"  Total rows: {total_rows}")

    if failed:
        print(f"\nFailed files:")
        for r in failed:
            print(f"  - {r['file']}: {r.get('error', 'Unknown error')}")

    print("=" * 80)

    if failed:
        print("❌ Some files failed to read")
        return 1
    else:
        print("✅ All files read successfully!")
        return 0


if __name__ == "__main__":
    exit(main())
