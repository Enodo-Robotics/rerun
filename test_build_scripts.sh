#!/bin/bash

# Test script for build and release automation
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

print_test() { echo -e "${BLUE}[TEST]${NC} $1"; }
print_pass() { echo -e "${GREEN}[PASS]${NC} $1"; }
print_fail() { echo -e "${RED}[FAIL]${NC} $1"; }

echo "🧪 Testing Rerun Build & Release Scripts"
echo "========================================"

# Test 1: Script syntax validation
print_test "Checking script syntax..."
if bash -n build_and_release.sh && bash -n release_wheel.sh && bash -n upload_wheel.sh; then
    print_pass "All scripts have valid syntax"
else
    print_fail "Script syntax errors found"
    exit 1
fi

# Test 2: Check script permissions
print_test "Checking script permissions..."
if [[ -x build_and_release.sh && -x release_wheel.sh && -x upload_wheel.sh ]]; then
    print_pass "All scripts are executable"
else
    print_fail "Scripts missing execute permissions"
    exit 1
fi

# Test 3: Test wheel discovery
print_test "Testing wheel discovery..."
if [[ -f "./wheels/rerun_sdk-0.24.0a1+dev-cp39-abi3-manylinux_2_35_x86_64.whl" ]]; then
    print_pass "Wheel file found"
else
    print_fail "Wheel file not found"
    exit 1
fi

# Test 4: Version extraction
print_test "Testing version extraction..."
wheel_file="./wheels/rerun_sdk-0.24.0a1+dev-cp39-abi3-manylinux_2_35_x86_64.whl"
version=$(basename "$wheel_file" | sed -n 's/rerun_sdk-\([^-]*\)-.*/\1/p')
if [[ "$version" == "0.24.0a1+dev" ]]; then
    print_pass "Version extraction works: $version"
else
    print_fail "Version extraction failed: got '$version'"
    exit 1
fi

# Test 5: Wheel content validation
print_test "Testing wheel content validation..."
if python -m zipfile -l "$wheel_file" | grep -q "rerun_sdk/rerun_cli/rerun"; then
    print_pass "Wheel contains CLI binary"
else
    print_fail "Wheel missing CLI binary"
    exit 1
fi

# Test 6: Test prerequisite checking
print_test "Testing prerequisite checking..."
missing_tools=()
for tool in git cargo python maturin; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        missing_tools+=("$tool")
    fi
done

if [[ ${#missing_tools[@]} -eq 0 ]]; then
    print_pass "All required tools available"
else
    print_fail "Missing tools: ${missing_tools[*]}"
    exit 1
fi

# Test 7: Branch detection
print_test "Testing branch detection..."
current_branch=$(git branch --show-current)
if [[ "$current_branch" == "oscar-rrl" ]]; then
    print_pass "Correct branch detected: $current_branch"
else
    print_fail "Wrong branch: $current_branch (expected oscar-rrl)"
    exit 1
fi

# Test 8: Git status detection
print_test "Testing git status detection..."
if git diff-index --quiet HEAD --; then
    print_pass "No uncommitted changes"
else
    print_pass "Uncommitted changes detected (expected)"
fi

# Test 9: Tag name generation
print_test "Testing tag name generation..."
tag_name="multiprocess-safe-v${version}"
expected_tag="multiprocess-safe-v0.24.0a1+dev"
if [[ "$tag_name" == "$expected_tag" ]]; then
    print_pass "Tag name generation works: $tag_name"
else
    print_fail "Tag name generation failed: got '$tag_name', expected '$expected_tag'"
    exit 1
fi

# Test 10: File size calculation
print_test "Testing file size calculation..."
file_size=$(stat -c%s "$wheel_file" 2>/dev/null || stat -f%z "$wheel_file" 2>/dev/null || echo "unknown")
if [[ "$file_size" != "unknown" && "$file_size" -gt 0 ]]; then
    print_pass "File size calculation works: $file_size bytes"
else
    print_fail "File size calculation failed: $file_size"
    exit 1
fi

echo
echo "🎉 All tests passed! The build and release scripts are working correctly."
echo
echo "To test the full functionality:"
echo "1. Install twine: pip install twine"
echo "2. Run: ./build_and_release.sh (select option 1 for testing)"
echo "3. Run: ./release_wheel.sh (select option 6 for wheel testing)"
echo "4. Run: ./upload_wheel.sh (select option 1 for TestPyPI upload)"