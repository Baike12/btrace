#!/bin/bash
# ============================================================================
# Langfuse Rust Backend — Dev Startup Script
# ============================================================================
set -euo pipefail
cd "$(dirname "$0")/.."

echo "=== Langfuse Rust Backend ==="

# Load .env if exists
if [ -f .env ]; then
    export $(grep -v '^#' .env | xargs)
fi

# Ensure DATABASE_URL is set
: "${DATABASE_URL:?DATABASE_URL must be set}"

echo "Database: ${DATABASE_URL}"
echo ""

MODE=${1:-server}

case "$MODE" in
    worker)
        echo "Starting queue consumer..."
        cargo run --bin worker
        ;;
    server)
        echo "Starting API server + queue consumer..."
        cargo run --bin server
        ;;
    check)
        echo "Running cargo check..."
        cargo check
        ;;
    test)
        echo "Running tests..."
        cargo test --all
        ;;
    clippy)
        echo "Running clippy..."
        cargo clippy --all-targets
        ;;
    build)
        echo "Building release..."
        cargo build --release
        ;;
    *)
        echo "Usage: $0 {server|worker|check|test|clippy|build}"
        exit 1
        ;;
esac
