#!/bin/bash
# Axe'n'Stax — Run the game
#
# Run this on the HOST machine (not inside VirtualBox).
# The host has the real GPU needed for rendering.
#
# Usage:
#   ./run.sh          — run the game
#   ./run.sh build    — rebuild from source then run

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BINARY="$SCRIPT_DIR/build/axenstax-engine"

if [ "$1" = "build" ]; then
    echo "Building Axe'n'Stax (release)..."
    if ! command -v cargo &> /dev/null; then
        echo "Rust not installed. Install from: https://rustup.rs"
        exit 1
    fi
    cd "$SCRIPT_DIR"
    cargo build --release --target-dir "$SCRIPT_DIR/build"
    echo "Build complete."
fi

if [ ! -f "$BINARY" ]; then
    # Try the VM-built binary location
    BINARY="$SCRIPT_DIR/build/release/axenstax-engine"
fi

if [ ! -f "$BINARY" ]; then
    echo "No binary found. Run: ./run.sh build"
    echo "Or install Rust from https://rustup.rs first."
    exit 1
fi

echo "=== Axe'n'Stax ==="
echo "Controls: WASD=move, Mouse=look, Space=jump, Ctrl=sprint"
echo "          Double-tap Space=fly, F3=debug, Escape=quit"
echo ""
exec "$BINARY"
