# Command executor: runs CI job commands as subprocesses. git/tar/curl are
# in the image because pipeline jobs (e.g. .orchestrator/actions.yml) use
# them to clone and package the repo under test.
FROM ghcr.io/astral-sh/uv:python3.13-bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends git tar curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# docker CLI + compose so deploy jobs can drive the host daemon when
# /var/run/docker.sock is mounted (self-deploy) — inert without the socket
RUN install -m 0755 -d /etc/apt/keyrings \
    && curl -fsSL https://download.docker.com/linux/debian/gpg -o /etc/apt/keyrings/docker.asc \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/debian bookworm stable" \
       > /etc/apt/sources.list.d/docker.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends docker-ce-cli docker-compose-plugin \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY pyproject.toml uv.lock ./
RUN uv sync --no-dev
COPY app app

EXPOSE 9000
CMD ["uv", "run", "--no-sync", "uvicorn", "app.main:app", "--host", "0.0.0.0", "--port", "9000"]
