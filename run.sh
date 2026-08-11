#!/bin/bash
cargo run -- worker --name beefy-1 --tags heavy,docker --coordinator https://ci.manishacharya.name.np

cd command-executor && uv run uvicorn app.main:app --port 9000
