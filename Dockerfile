# Coordinator + worker image (same binary, different subcommand).
# The coordinator also serves the built dashboard and runs the yaml-parser
# through uv, so this one image is UI + API + planner.

# ---- dashboard bundle -------------------------------------------------
FROM node:22-alpine AS dashboard
WORKDIR /build
COPY dashboard/package.json dashboard/package-lock.json ./
RUN npm ci
COPY dashboard .
RUN npm run build

# ---- rust binary ------------------------------------------------------
FROM rust:1-bookworm AS rust
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src src
RUN cargo build --release

# ---- runtime ----------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=ghcr.io/astral-sh/uv:latest /uv /usr/local/bin/uv

WORKDIR /app
# yaml-parser runs via its own uv-managed venv (uv fetches python itself)
COPY yaml-parser/pyproject.toml yaml-parser/uv.lock yaml-parser/
COPY yaml-parser/*.py yaml-parser/
RUN cd yaml-parser && uv sync --no-dev

COPY --from=rust /build/target/release/orchestrator /usr/local/bin/orchestrator
COPY --from=dashboard /build/dist /app/dashboard/dist
# this repo's own pipeline — the trigger fallback until it's pushed to Forgejo
COPY .orchestrator .orchestrator

ENV YAML_PARSER_DIR=/app/yaml-parser \
    YAML_PARSER_PYTHON=/app/yaml-parser/.venv/bin/python \
    DASHBOARD_DIST=/app/dashboard/dist

EXPOSE 8080
CMD ["orchestrator", "coordinator", "--port", "8080"]
