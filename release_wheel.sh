#!/bin/bash

# Rerun Custom Wheel Release Script
# This script creates a git tag and uploads the custom wheel to PyPI

set -euo pipefail  # Exit on error, undefined variables, and pipe failures

# Configuration
WHEEL_DIR="./wheels"
BRANCH="oscar-rrl"
REPO_URL="https://github.com/rerun-io/rerun"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Function to print colored output
print_status() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Function to check if command exists
command_exists() {
    command -v "$1" >/dev/null 2>&1
}

# Function to get the latest wheel file
get_latest_wheel() {
    local wheel_file=$(ls -t "${WHEEL_DIR}"/rerun_sdk-*.whl 2>/dev/null | head -1)
    if [[ -z "$wheel_file" ]]; then
        print_error "No wheel files found in ${WHEEL_DIR}"
        return 1
    fi
    echo "$wheel_file"
}

# Function to extract version from wheel filename
extract_version_from_wheel() {
    local wheel_file="$1"
    local basename=$(basename "$wheel_file")
    # Extract version from filename like "rerun_sdk-0.24.0a1+dev-cp39-abi3-manylinux_2_35_x86_64.whl"
    local version=$(echo "$basename" | sed -n 's/rerun_sdk-\([^-]*\)-.*/\1/p')
    echo "$version"
}

# Function to create git tag
create_git_tag() {
    local version="$1"
    local tag_name="multiprocess-safe-v${version}"
    
    print_status "Creating git tag: $tag_name"
    
    # Check if tag already exists
    if git tag -l | grep -q "^${tag_name}$"; then
        print_warning "Tag $tag_name already exists"
        read -p "Do you want to delete and recreate it? (y/N): " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            git tag -d "$tag_name"
            git push origin --delete "$tag_name" 2>/dev/null || true
        else
            print_error "Aborting due to existing tag"
            return 1
        fi
    fi
    
    # Create annotated tag with release notes
    local tag_message="Multiprocess-Safe Rerun Release v${version}

This release includes custom multiprocess-safe file handling:
- Zero-contention multiprocess writes
- Process-specific recording files (filename_pid12345.rrd)
- Headless mode support with --save-interval
- File rotation with --rotate-files
- All original Rerun functionality preserved

Built from branch: ${BRANCH}
Wheel: $(basename "$(get_latest_wheel)")

🚀 Ready for production use with unlimited concurrent processes!"

    git tag -a "$tag_name" -m "$tag_message"
    
    print_success "Created tag: $tag_name"
    return 0
}

# Function to push tag to remote
push_tag() {
    local version="$1"
    local tag_name="multiprocess-safe-v${version}"
    
    print_status "Pushing tag to remote repository..."
    
    # Check if we have a remote configured
    if ! git remote get-url origin >/dev/null 2>&1; then
        print_error "No git remote 'origin' configured"
        return 1
    fi
    
    git push origin "$tag_name"
    print_success "Tag pushed to remote repository"
}

# Function to upload wheel to PyPI
upload_to_pypi() {
    local wheel_file="$1"
    local repository="$2"
    
    print_status "Uploading wheel to $repository..."
    print_status "Wheel: $(basename "$wheel_file")"
    
    # Upload the wheel
    if [[ "$repository" == "testpypi" ]]; then
        twine upload --repository testpypi "$wheel_file"
    else
        twine upload "$wheel_file"
    fi
    
    print_success "Wheel uploaded to $repository"
}

# Function to verify wheel before upload
verify_wheel() {
    local wheel_file="$1"
    
    print_status "Verifying wheel: $(basename "$wheel_file")"
    
    # Check if wheel file exists and is readable
    if [[ ! -f "$wheel_file" ]]; then
        print_error "Wheel file not found: $wheel_file"
        return 1
    fi
    
    if [[ ! -r "$wheel_file" ]]; then
        print_error "Wheel file not readable: $wheel_file"
        return 1
    fi
    
    # Check wheel contents
    if command_exists python; then
        python -m zipfile -l "$wheel_file" | grep -q "rerun_sdk/rerun_cli/rerun" || {
            print_error "Wheel does not contain expected CLI binary"
            return 1
        }
    fi
    
    local file_size=$(stat -c%s "$wheel_file" 2>/dev/null || stat -f%z "$wheel_file" 2>/dev/null || echo "unknown")
    print_success "Wheel verified (size: $file_size bytes)"
}

# Function to test wheel installation
test_wheel() {
    local wheel_file="$1"
    
    print_status "Testing wheel installation..."
    
    # Create a temporary virtual environment for testing
    local temp_venv=$(mktemp -d)
    trap "rm -rf $temp_venv" EXIT
    
    python -m venv "$temp_venv"
    source "$temp_venv/bin/activate"
    
    # Install the wheel
    pip install "$wheel_file"
    
    # Test that it imports correctly
    python -c "import rerun; print('✓ Rerun imports successfully')"
    
    # Test that the CLI works
    python -c "
import subprocess
result = subprocess.run(['python', '-m', 'rerun', '--help'], 
                       capture_output=True, text=True)
if 'headless' in result.stdout.lower():
    print('✓ CLI has headless features')
else:
    print('✗ CLI missing headless features')
    exit(1)
"
    
    deactivate
    print_success "Wheel installation test passed"
}

# Main function
main() {
    print_status "Starting Rerun Custom Wheel Release Process"
    print_status "Repository: $REPO_URL"
    print_status "Branch: $BRANCH"
    
    # Check prerequisites
    print_status "Checking prerequisites..."
    
    if ! command_exists git; then
        print_error "git is not installed"
        exit 1
    fi
    
    if ! command_exists python; then
        print_error "python is not installed"
        exit 1
    fi
    
    if ! command_exists twine; then
        print_error "twine is not installed. Install with: pip install twine"
        exit 1
    fi
    
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
    
    # Check for uncommitted changes
    if ! git diff-index --quiet HEAD --; then
        print_warning "You have uncommitted changes"
        read -p "Continue anyway? (y/N): " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            print_error "Aborting. Commit your changes first."
            exit 1
        fi
    fi
    
    # Get the latest wheel
    print_status "Finding latest wheel..."
    wheel_file=$(get_latest_wheel)
    version=$(extract_version_from_wheel "$wheel_file")
    
    if [[ -z "$version" ]]; then
        print_error "Could not extract version from wheel filename"
        exit 1
    fi
    
    print_success "Found wheel: $(basename "$wheel_file")"
    print_success "Version: $version"
    
    # Verify the wheel
    verify_wheel "$wheel_file"
    
    # Ask user what they want to do
    echo
    print_status "What would you like to do?"
    echo "1) Create git tag only"
    echo "2) Create git tag and push to remote"
    echo "3) Upload wheel to TestPyPI"
    echo "4) Upload wheel to PyPI (production)"
    echo "5) Full release (tag + push + PyPI upload)"
    echo "6) Test wheel installation"
    echo "0) Exit"
    
    read -p "Enter your choice (0-6): " -n 1 -r choice
    echo
    
    case $choice in
        1)
            create_git_tag "$version"
            ;;
        2)
            create_git_tag "$version" && push_tag "$version"
            ;;
        3)
            upload_to_pypi "$wheel_file" "testpypi"
            ;;
        4)
            print_warning "This will upload to production PyPI!"
            read -p "Are you sure? (y/N): " -n 1 -r
            echo
            if [[ $REPLY =~ ^[Yy]$ ]]; then
                upload_to_pypi "$wheel_file" "pypi"
            else
                print_status "Upload cancelled"
            fi
            ;;
        5)
            print_warning "This will create a tag, push it, and upload to production PyPI!"
            read -p "Are you sure? (y/N): " -n 1 -r
            echo
            if [[ $REPLY =~ ^[Yy]$ ]]; then
                create_git_tag "$version" && \
                push_tag "$version" && \
                upload_to_pypi "$wheel_file" "pypi"
            else
                print_status "Full release cancelled"
            fi
            ;;
        6)
            test_wheel "$wheel_file"
            ;;
        0)
            print_status "Exiting..."
            exit 0
            ;;
        *)
            print_error "Invalid choice"
            exit 1
            ;;
    esac
    
    print_success "Release process completed successfully!"
}

# Run main function
main "$@"