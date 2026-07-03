#!/bin/bash
# Bundle yt-dlp binary for Curio Reader

set -e

YTDLP_VERSION="${YTDLP_VERSION:-2024.12.23}"
BIN_DIR="src-tauri/bin"
FORCE="${1:-}"

mkdir -p "$BIN_DIR"

# Detect platform and set appropriate binary name
case "$(uname -s)" in
    Darwin*)
        PLATFORM="macos"
        case "$(uname -m)" in
            arm64)
                YTDLP_URL="https://github.com/yt-dlp/yt-dlp/releases/download/${YTDLP_VERSION}/yt-dlp_macos"
                YTDLP_BIN="$BIN_DIR/yt-dlp-aarch64-apple-darwin"
                ;;
            x86_64)
                YTDLP_URL="https://github.com/yt-dlp/yt-dlp/releases/download/${YTDLP_VERSION}/yt-dlp_macos"
                YTDLP_BIN="$BIN_DIR/yt-dlp-x86_64-apple-darwin"
                ;;
            *)
                echo "Unsupported macOS architecture: $(uname -m)"
                exit 1
                ;;
        esac
        ;;
    Linux*)
        PLATFORM="linux"
        YTDLP_URL="https://github.com/yt-dlp/yt-dlp/releases/download/${YTDLP_VERSION}/yt-dlp"
        YTDLP_BIN="$BIN_DIR/yt-dlp-x86_64-unknown-linux-gnu"
        ;;
    MINGW*|CYGWIN*|MSYS*)
        PLATFORM="windows"
        YTDLP_URL="https://github.com/yt-dlp/yt-dlp/releases/download/${YTDLP_VERSION}/yt-dlp.exe"
        YTDLP_BIN="$BIN_DIR/yt-dlp-x86_64-pc-windows-msvc.exe"
        ;;
    *)
        echo "Unsupported operating system: $(uname -s)"
        exit 1
        ;;
esac

echo "Platform: $PLATFORM"
echo "Target: $YTDLP_BIN"

# Check if already installed
if [ -f "$YTDLP_BIN" ] && [ "$FORCE" != "--force" ]; then
    CURRENT_VERSION=$("$YTDLP_BIN" --version 2>/dev/null || echo "unknown")
    echo "yt-dlp already installed: $CURRENT_VERSION"
    echo "Use --force to re-download"
    exit 0
fi

# Download yt-dlp
echo "Downloading yt-dlp ${YTDLP_VERSION}..."
if command -v curl &> /dev/null; then
    curl -L "$YTDLP_URL" -o "$YTDLP_BIN" --progress-bar
elif command -v wget &> /dev/null; then
    wget "$YTDLP_URL" -O "$YTDLP_BIN" --show-progress
else
    echo "Error: curl or wget is required"
    exit 1
fi

# Make executable
chmod +x "$YTDLP_BIN"

# Verify installation
if [ -x "$YTDLP_BIN" ]; then
    VERSION=$("$YTDLP_BIN" --version 2>/dev/null || echo "unknown")
    echo "yt-dlp installed successfully: $VERSION"
    echo "Location: $YTDLP_BIN"
else
    echo "Error: Failed to install yt-dlp"
    exit 1
fi
