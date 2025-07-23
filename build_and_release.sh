#!/bin/bash

# Complete Build and Release Script for Rerun Custom Wheel
# This script builds the wheel from scratch and optionally releases it

set -euo pipefail

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m'

print_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
print_success() { echo -e "${GREEN}[SUCCESS]${NC} $1"; }
print_error() { echo -e "${RED}[ERROR]${NC} $1"; }
print_warning() { echo -e "${YELLOW}[WARNING]${NC} $1"; }

# Configuration
BRANCH="oscar-rrl"
WHEEL_DIR="./wheels"

# Main function
main() {
    print_info "🚀 Rerun Custom Wheel Build and Release Script"
    print_info "Branch: $BRANCH"
    
    # Check prerequisites
    print_info "Checking prerequisites..."
    
    for cmd in git cargo python maturin; do
        if ! command -v "$cmd" >/dev/null 2>&1; then
            print_error "$cmd is not installed"
            exit 1
        fi
    done
    
    # Check if we're on the correct branch
    current_branch=$(git branch --show-current)
    if [[ "$current_branch" != "$BRANCH" ]]; then
        print_warning "Currently on branch '$current_branch', expected '$BRANCH'"
        read -p "Continue anyway? (y/N): " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            print_error "Aborting. Switch to branch '$BRANCH' first."
            exit 1
        fi
    fi
    
    print_success "Prerequisites check passed"
    
    # Show current status
    print_info "Current status:"
    print_info "  Branch: $(git branch --show-current)"
    print_info "  Latest commit: $(git log --oneline -1)"
    
    # Create wheels directory if it doesn't exist
    mkdir -p "$WHEEL_DIR"
    
    # Build process
    print_info "🔨 Starting build process..."
    
    print_info "Step 1: Building CLI binary..."
    if cargo build --release --bin rerun --manifest-path crates/top/rerun-cli/Cargo.toml; then
        print_success "CLI binary built successfully"
    else
        print_error "Failed to build CLI binary"
        exit 1
    fi
    
    print_info "Step 2: Copying CLI binary to Python package..."
    if cp ./target/release/rerun ./rerun_py/rerun_sdk/rerun_cli/rerun; then
        print_success "CLI binary copied successfully"
    else
        print_error "Failed to copy CLI binary"
        exit 1
    fi
    
    print_info "Step 3: Building Python wheel..."
    if RERUN_BUILDING_WHEEL=1 maturin build --release --manifest-path rerun_py/Cargo.toml --features web_viewer,nasm --out "$WHEEL_DIR/"; then
        print_success "Python wheel built successfully"
    else
        print_error "Failed to build Python wheel"
        exit 1
    fi
    
    # Find the built wheel
    local wheel_file=$(ls -t "${WHEEL_DIR}"/rerun_sdk-*.whl 2>/dev/null | head -1)
    if [[ -z "$wheel_file" ]]; then
        print_error "No wheel file found after build"
        exit 1
    fi
    
    local wheel_name=$(basename "$wheel_file")
    local file_size=$(stat -c%s "$wheel_file" 2>/dev/null || stat -f%z "$wheel_file" 2>/dev/null || echo "unknown")
    
    print_success "🎉 Build completed successfully!"
    print_success "Wheel: $wheel_name"
    print_success "Size: $file_size bytes"
    
    # Ask what to do next
    echo
    print_info "What would you like to do next?"
    echo "1) Test the wheel locally"
    echo "2) Create git tag"
    echo "3) Upload to TestPyPI"
    echo "4) Upload to PyPI (production)"
    echo "5) Full release (tag + PyPI upload)"
    echo "0) Exit"
    
    read -p "Enter your choice (0-5): " -n 1 -r choice
    echo
    
    case $choice in
        1)
            print_info "Testing wheel locally..."
            test_wheel "$wheel_file"
            ;;
        2)
            create_git_tag "$wheel_file"
            ;;
        3)
            upload_to_testpypi "$wheel_file"
            ;;
        4)
            upload_to_pypi "$wheel_file"
            ;;
        5)
            full_release "$wheel_file"
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

# Test wheel function
test_wheel() {
    local wheel_file="$1"
    print_info "Installing wheel for testing..."
    
    pip install --force-reinstall "$wheel_file"
    
    print_info "Testing basic functionality..."
    python -c "
import rerun
print('✓ Rerun imports successfully')

# Test CLI
import subprocess
result = subprocess.run(['python', '-c', 'import rerun; help(rerun.save)'], 
                       capture_output=True, text=True)
if result.returncode == 0:
    print('✓ Basic API works')
else:
    print('✗ API test failed')
    exit(1)
"
    
    print_success "Wheel test passed!"
}

# Create git tag
create_git_tag() {
    local wheel_file="$1"
    local version=$(basename "$wheel_file" | sed -n 's/rerun_sdk-\([^-]*\)-.*/\1/p')
    local tag_name="multiprocess-safe-v${version}"
    
    print_info "Creating git tag: $tag_name"
    
    if git tag -l | grep -q "^${tag_name}$"; then
        print_warning "Tag already exists"
        return 1
    fi
    
    local tag_message="Multiprocess-Safe Rerun Release v${version}

Features:
- Zero-contention multiprocess writes
- Process-specific recording files
- Headless mode support
- File rotation support

Built wheel: $(basename "$wheel_file")"
    
    git tag -a "$tag_name" -m "$tag_message"
    print_success "Created tag: $tag_name"
    
    read -p "Push tag to remote? (y/N): " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        git push origin "$tag_name"
        print_success "Tag pushed to remote"
    fi
}

# Upload to TestPyPI
upload_to_testpypi() {
    local wheel_file="$1"
    print_info "Uploading to TestPyPI..."
    
    if ! command -v twine >/dev/null 2>&1; then
        print_error "twine is not installed. Install with: pip install twine"
        return 1
    fi
    
    twine upload --repository testpypi "$wheel_file"
    print_success "Uploaded to TestPyPI!"
    print_info "Install with: pip install -i https://test.pypi.org/simple/ rerun-sdk"
}

# Upload to PyPI
upload_to_pypi() {
    local wheel_file="$1"
    print_warning "⚠️  This will upload to PRODUCTION PyPI!"
    read -p "Are you absolutely sure? (y/N): " -n 1 -r
    echo
    
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        print_info "Uploading to PyPI..."
        
        if ! command -v twine >/dev/null 2>&1; then
            print_error "twine is not installed. Install with: pip install twine"
            return 1
        fi
        
        twine upload "$wheel_file"
        print_success "Uploaded to PyPI!"
        print_info "Install with: pip install rerun-sdk"
    else
        print_info "Upload cancelled"
    fi
}

# Full release
full_release() {
    local wheel_file="$1"
    print_warning "⚠️  This will create a tag AND upload to PyPI!"
    read -p "Are you absolutely sure? (y/N): " -n 1 -r
    echo
    
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        create_git_tag "$wheel_file" && upload_to_pypi "$wheel_file"
        print_success "🎉 Full release completed!"
    else
        print_info "Full release cancelled"
    fi
}

# Run main function
main "$@"