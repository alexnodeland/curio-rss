#!/bin/bash
# Generate TypeScript types from Rust structs using ts-rs

set -e

echo "Generating TypeScript types from Rust..."

# Create output directory
OUTPUT_DIR="src/lib/types/generated"
mkdir -p "$OUTPUT_DIR"

# Check if ts-rs feature is available
cd src-tauri

if grep -q 'ts-rs' Cargo.toml; then
    echo "Running type export tests..."
    cargo test export_bindings --features ts-rs -- --nocapture 2>/dev/null || {
        echo "Note: ts-rs feature not configured or no export tests found"
        echo "Skipping automatic type generation"
        cd ..
        exit 0
    }
else
    echo "Note: ts-rs not found in Cargo.toml"
    echo ""
    echo "To enable automatic type generation, add to Cargo.toml:"
    echo ""
    echo '[dependencies]'
    echo 'ts-rs = { version = "8", optional = true }'
    echo ''
    echo '[features]'
    echo 'ts-rs = ["dep:ts-rs"]'
    echo ""
    echo "Then add #[derive(TS)] and #[ts(export)] to your structs"
    cd ..
    exit 0
fi

cd ..

# Format generated types
if [ -d "$OUTPUT_DIR" ] && [ "$(ls -A $OUTPUT_DIR 2>/dev/null)" ]; then
    echo "Formatting generated types..."
    npx biome format --write "$OUTPUT_DIR" 2>/dev/null || true
    echo "Types generated in $OUTPUT_DIR"
else
    echo "No types were generated"
fi
