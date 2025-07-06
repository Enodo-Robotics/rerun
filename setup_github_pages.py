#!/usr/bin/env python3
"""
Script to automate GitHub Pages setup for pip index.
Requires GitHub CLI (gh) or manual steps.
"""

import subprocess
import sys
from pathlib import Path

def run_command(cmd, capture_output=True):
    """Run a shell command and return the result."""
    try:
        result = subprocess.run(cmd, shell=True, capture_output=capture_output, text=True)
        return result.returncode == 0, result.stdout, result.stderr
    except Exception as e:
        return False, "", str(e)

def setup_github_pages():
    """Set up GitHub Pages for the pip index."""
    
    print("🚀 Setting up GitHub Pages for pip index...")
    
    # Check if we have the index.html file
    index_file = Path("index.html")
    if not index_file.exists():
        print("❌ index.html not found. Run generate_pip_index.py first.")
        return False
    
    # Check current branch
    success, current_branch, _ = run_command("git branch --show-current")
    if not success:
        print("❌ Not in a git repository")
        return False
    
    current_branch = current_branch.strip()
    print(f"📍 Current branch: {current_branch}")
    
    # Create and switch to gh-pages branch
    print("🌿 Creating gh-pages branch...")
    success, _, _ = run_command("git checkout --orphan gh-pages")
    if not success:
        print("⚠️  gh-pages branch might already exist, trying to switch...")
        success, _, _ = run_command("git checkout gh-pages")
        if not success:
            print("❌ Failed to switch to gh-pages branch")
            return False
    
    # Remove all files except index.html
    print("🧹 Cleaning gh-pages branch...")
    run_command("git rm -rf .", capture_output=True)  # Remove tracked files
    
    # Add only the index.html
    print("📄 Adding index.html...")
    success, _, _ = run_command("git add index.html")
    if not success:
        print("❌ Failed to add index.html")
        return False
    
    # Commit
    print("💾 Committing index.html...")
    success, _, _ = run_command('git commit -m "Add pip index for rerun-sdk"')
    if not success:
        print("❌ Failed to commit")
        return False
    
    # Push to origin
    print("🚀 Pushing to GitHub...")
    success, _, stderr = run_command("git push origin gh-pages")
    if not success:
        print(f"❌ Failed to push: {stderr}")
        print("💡 You may need to push manually: git push origin gh-pages")
        return False
    
    print("✅ GitHub Pages setup complete!")
    print("\n📋 Final steps:")
    print("1. Go to: https://github.com/enodo-robotics/rerun/settings/pages")
    print("2. Set Source to 'Deploy from a branch'")
    print("3. Select 'gh-pages' branch and '/ (root)' folder")
    print("4. Save")
    print("\n🎉 After GitHub Pages is enabled, users can install with:")
    print("pip install --pre --no-index -f https://enodo-robotics.github.io/rerun/ rerun-sdk")
    
    # Switch back to original branch
    print(f"\n🔄 Switching back to {current_branch}...")
    run_command(f"git checkout {current_branch}")
    
    return True

def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--help":
        print("Usage: python3 setup_github_pages.py")
        print("This script sets up GitHub Pages for your pip index.")
        print("Make sure you have git configured and are in the repo directory.")
        return
    
    setup_github_pages()

if __name__ == "__main__":
    main()