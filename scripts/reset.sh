#!/usr/bin/env sh
# Wipe the orchestrator's history so the site starts fresh.
#
# Deliberately NOT part of self-deploy. A deploy that silently deleted history
# would make every push destructive, and you would only notice the first time
# you wanted to look something up. Run this by hand when you actually want a
# clean slate.
#
#   ./scripts/reset.sh                 # runs + jobs + logs + artifacts, keeps repos
#   ./scripts/reset.sh --all           # the above AND unregisters every repository
#   ./scripts/reset.sh --yes           # skip the confirmation prompt
#
# Works from any directory, with or without passwordless docker.
set -eu

KEEP_REPOS=1
ASSUME_YES=0
for arg in "$@"; do
  case "$arg" in
    --all) KEEP_REPOS=0 ;;
    --yes|-y) ASSUME_YES=1 ;;
    -h|--help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $arg (try --help)" >&2; exit 2 ;;
  esac
done

# Find the stack from this script's own location rather than the caller's cwd —
# running it from scripts/ is the obvious thing to do, and `docker compose` only
# discovers the compose file (and its override) in the directory it runs from.
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
STACK_DIR=${STACK_DIR:-$(dirname -- "$SCRIPT_DIR")}
if [ ! -f "$STACK_DIR/docker-compose.yml" ]; then
  echo "No docker-compose.yml in $STACK_DIR." >&2
  echo "Set STACK_DIR=/path/to/stack and try again." >&2
  exit 1
fi
cd "$STACK_DIR"

# The server may or may not let this user talk to the docker socket directly.
DOCKER=docker
if ! docker info >/dev/null 2>&1; then
  if command -v sudo >/dev/null 2>&1 && sudo docker info >/dev/null 2>&1; then
    DOCKER="sudo docker"
    echo "(using sudo for docker)"
  else
    echo "Cannot talk to the Docker daemon as $(id -un)." >&2
    echo "Either run this with sudo, or add yourself to the docker group." >&2
    exit 1
  fi
fi

# compose v2 plugin, falling back to the standalone binary
if $DOCKER compose version >/dev/null 2>&1; then
  DC="$DOCKER compose"
elif command -v docker-compose >/dev/null 2>&1; then
  DC="docker-compose"
else
  echo "Neither 'docker compose' nor 'docker-compose' is available." >&2
  exit 1
fi

# `docker compose exec -T` inherits stdin and will happily eat the confirmation
# we are about to read, so every call here gets /dev/null explicitly. Losing the
# prompt on a script whose whole job is deleting things is not a small bug.
psql_q() { $DC exec -T postgres psql -U orchestrator -d orchestrator -qtA -v ON_ERROR_STOP=1 "$@" </dev/null; }
redis_q() { $DC exec -T redis redis-cli "$@" </dev/null; }

# Say which thing is actually wrong instead of one generic "is the stack up?".
if ! $DC ps --status running --services 2>/dev/null | grep -qx postgres; then
  echo "The postgres service is not running in $STACK_DIR." >&2
  echo "Start it with:  cd $STACK_DIR && $DC up -d" >&2
  exit 1
fi
if ! runs=$(psql_q -c "SELECT count(*) FROM runs;" 2>&1); then
  echo "Postgres is running but the query failed:" >&2
  echo "$runs" | sed 's/^/  /' >&2
  exit 1
fi
repos=$(psql_q -c "SELECT count(*) FROM repos;" 2>/dev/null || echo "?")

echo "Stack: $STACK_DIR"
echo "This will permanently delete:"
echo "  - $runs run(s), every job under them, and their captured logs"
echo "  - stored artifacts"
echo "  - the worker registry, job queues and CPU/RAM history"
echo "  - webhook delivery records and schedule state"
if [ "$KEEP_REPOS" -eq 0 ]; then
  echo "  - all $repos registered repositories  [--all]"
else
  echo "Keeping $repos registered repositories. Use --all to drop those too."
fi

if [ "$ASSUME_YES" -eq 0 ]; then
  printf 'Type "reset" to continue: '
  read -r reply
  [ "$reply" = "reset" ] || { echo "aborted — nothing was deleted"; exit 1; }
fi

# jobs cascade from runs (ON DELETE CASCADE), so this one statement takes the
# logs with it; the sequence restart is what makes the next run #1 again
psql_q -c "SET client_min_messages = warning; TRUNCATE runs RESTART IDENTITY CASCADE;" >/dev/null
psql_q -c "TRUNCATE webhook_deliveries;" >/dev/null
psql_q -c "TRUNCATE schedule_state;" >/dev/null
[ "$KEEP_REPOS" -eq 0 ] && psql_q -c "TRUNCATE repos;" >/dev/null

# Redis holds live state, not history: the worker registry, the ready-job
# queues, and each worker's rolling stats. Leaving the queues behind would
# strand ids that no longer exist in Postgres.
redis_q DEL workers jobs:ready >/dev/null 2>&1 || true
for pattern in 'jobs:ready:*' 'workers:stats:*'; do
  redis_q --scan --pattern "$pattern" 2>/dev/null | tr -d '\r' | while read -r key; do
    [ -n "$key" ] && redis_q DEL "$key" >/dev/null 2>&1 || true
  done
done

# Artifacts are files, not rows — TRUNCATE does not touch them.
$DC exec -T coordinator sh -c 'rm -rf "${ARTIFACTS_DIR:-/app/artifacts}"/* 2>/dev/null' </dev/null >/dev/null 2>&1 || true

# Workers cache their id on disk so they keep one identity across restarts.
# That is right in normal operation and wrong here: the registry was just
# emptied, so the running worker must register afresh rather than heartbeat
# against an id nothing knows about.
$DC restart worker >/dev/null 2>&1 || true

echo
echo "Done. Registry and history cleared; the next run will be #1."
echo "Workers re-register within a few seconds — reload the dashboard to confirm."
echo "Remote workers (your laptop) need a manual restart to re-register."
