#!/bin/bash
# First-time development setup for Curio Reader

set -e

echo "Setting up Curio Reader development environment..."
echo ""

# Check prerequisites
check_command() {
    if ! command -v "$1" &> /dev/null; then
        echo "Error: $1 is required but not installed."
        echo "$2"
        exit 1
    fi
}

echo "Checking prerequisites..."

check_command "rustc" "Install Rust from https://rustup.rs"
check_command "cargo" "Install Rust from https://rustup.rs"
check_command "node" "Install Node.js 20+ from https://nodejs.org"
check_command "npm" "Install Node.js 20+ from https://nodejs.org"

# Check Node version
NODE_VERSION=$(node -v | cut -d'v' -f2 | cut -d'.' -f1)
if [ "$NODE_VERSION" -lt 18 ]; then
    echo "Error: Node.js 18+ is required. Current version: $(node -v)"
    exit 1
fi

# Check Rust version
RUST_VERSION=$(rustc --version | cut -d' ' -f2 | cut -d'.' -f1-2)
echo "Rust version: $RUST_VERSION"

echo ""
echo "Installing Tauri CLI..."
cargo install tauri-cli 2>/dev/null || echo "Tauri CLI already installed or installation skipped"

echo ""
echo "Installing Node.js dependencies..."
npm install

echo ""
echo "Fetching Rust dependencies..."
cd src-tauri && cargo fetch && cd ..

echo ""
echo "Setting up git hooks..."
npx lefthook install

echo ""
echo "Bundling yt-dlp..."
./scripts/bundle-ytdlp.sh

echo ""
echo "============================================"
echo "Setup complete!"
echo ""
echo "To start developing, run:"
echo "  make dev"
echo ""
echo "Other useful commands:"
echo "  make test    - Run all tests"
echo "  make lint    - Run linters"
echo "  make build   - Build for production"
echo "  make help    - Show all commands"
echo "============================================"
