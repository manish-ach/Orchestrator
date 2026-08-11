#!/bin/bash

set -e

cd command-executor
uv run uvicorn app.main:app --port 9000 &
UVICORN_PID=$!
cd ..

cleanup() {
    kill "$UVICORN_PID" 2>/dev/null
}

trap cleanup EXIT INT TERM

cargo run -- worker \
    --name beefy-1 \
    --tags heavy,docker \
    --coordinator https://ci.manishacharya.name.np
