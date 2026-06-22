[Main repo at - https://git.manishacharya.name.np/Manish/Orchestrator.git]
# CI/CD Orchestrator

Distributed CI/CD orchestrator written in Rust. One binary, two modes:
a coordinator that hands out jobs and tracks workers, and workers that
run them.

This is the coordinator side (work in progress).

## Build

    cargo build

## Run

Start the coordinator:

    cargo run -- coordinator --port 8080

Register a worker (from another terminal):

    curl -X POST 127.0.0.1:8080/api/workers/register \
      -H 'Content-Type: application/json' \
      -d '{"worker_name": "WorkerA"}'

List registered workers:

    curl 127.0.0.1:8080/api/workers

## Endpoints

    GET  /              health text
    GET  /api/health    worker count
    POST /api/workers/register
    GET  /api/workers   list workers

## Status

Done: arg parsing, coordinator server, worker registry.
Next: worker client, job queue, heartbeats, reaper.
