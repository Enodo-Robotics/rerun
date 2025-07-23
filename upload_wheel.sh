#!/bin/bash

# Simple PyPI Upload Script for Rerun Custom Wheel

set -euo pipefail

# Configuration
WHEEL_DIR="./wheels"

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m'

print_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
print_success() { echo -e "${GREEN}[SUCCESS]${NC} $1"; }
print_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# Get the latest wheel
get_latest_wheel() {
    local wheel_file=$(ls -t "${WHEEL_DIR}"/rerun_sdk-*.whl 2>/dev/null | head -1)
    if [[ -z "$wheel_file" ]]; then
        print_error "No wheel files found in ${WHEEL_DIR}"
        exit 1
    fi
    echo "$wheel_file"
}

# Main script
main() {
    print_info "Rerun Custom Wheel Upload Script"
    
    # Check if twine is installed
    if ! command -v twine >/dev/null 2>&1; then
        print_error "twine is not installed. Install with: pip install twine"
        exit 1
    fi
    
    # Get the latest wheel
    local wheel_file=$(get_latest_wheel)
    local wheel_name=$(basename "$wheel_file")
    
    print_success "Found wheel: $wheel_name"
    
    # Show wheel info
    local file_size=$(stat -c%s "$wheel_file" 2>/dev/null || stat -f%z "$wheel_file" 2>/dev/null || echo "unknown")
    print_info "Size: $file_size bytes"
    
    # Ask user where to upload
    echo
    echo "Where would you like to upload the wheel?"
    echo "1) TestPyPI (recommended for testing)"
    echo "2) PyPI (production)"
    echo "0) Exit"
    
    read -p "Enter your choice (0-2): " -n 1 -r choice
    echo
    
    case $choice in
        1)
            print_info "Uploading to TestPyPI..."
            twine upload --repository testpypi "$wheel_file"
            print_success "Uploaded to TestPyPI!"
            echo "Install with: pip install -i https://test.pypi.org/simple/ rerun-sdk"
            ;;
        2)
            print_info "⚠️  This will upload to PRODUCTION PyPI!"
            read -p "Are you absolutely sure? (y/N): " -n 1 -r confirm
            echo
            if [[ $confirm =~ ^[Yy]$ ]]; then
                print_info "Uploading to PyPI..."
                twine upload "$wheel_file"
                print_success "Uploaded to PyPI!"
                echo "Install with: pip install rerun-sdk"
            else
                print_info "Upload cancelled"
            fi
            ;;
        0)
            print_info "Exiting..."
            exit 0
            ;;
        *)
            print_error "Invalid choice"
            exit 1
            ;;
    esac
}

main "$@"