#!/usr/bin/env bash
#
# Drives the coordinator through one full pipeline run:
#   trigger -> queue -> claim -> (run) -> report -> status
#
# Usage:
#   ./scripts/demo-pipeline.sh
#   BASE=http://127.0.0.1:8080 WORKER=WorkerA ./scripts/demo-pipeline.sh
#
set -euo pipefail

BASE="${BASE:-http://127.0.0.1:8080}"
WORKER="${WORKER:-WorkerA}"

# ---- colors ----
BOLD=$'\033[1m'; DIM=$'\033[2m'
GRN=$'\033[32m'; CYN=$'\033[36m'; YLW=$'\033[33m'; RED=$'\033[31m'; RST=$'\033[0m'

hr()   { printf '%s\n' "${DIM}────────────────────────────────────────────────────${RST}"; }
step() { printf '\n%s%s%s\n' "${BOLD}${CYN}" "$1" "${RST}"; hr; }

command -v jq >/dev/null 2>&1 || { echo "${RED}jq is required${RST} (brew install jq)"; exit 1; }

# ---- 0. reachable? ----
step "0. Coordinator reachable at ${BASE}?"
if curl -sf "${BASE}/" >/dev/null; then
  printf '   %sonline%s\n' "$GRN" "$RST"
else
  printf '   %scoordinator not running at %s%s\n' "$RED" "$BASE" "$RST"
  echo "   start it with:  cargo run -- coordinator --port ${BASE##*:}"
  exit 1
fi

# ---- 1. register a worker ----
step "1. Register worker (${WORKER})"
curl -sX POST "${BASE}/api/workers/register" \
  -H 'Content-Type: application/json' -d "{\"worker_name\":\"${WORKER}\"}"
printf '   %sregistered%s\n' "$GRN" "$RST"

# ---- 2. trigger the pipeline ----
step "2. Trigger pipeline"
curl -sX POST "${BASE}/api/pipelines/trigger"
printf '   %spipeline triggered%s\n' "$GRN" "$RST"

# ---- 3. show the queue ----
step "3. Queue — jobs after trigger"
curl -s "${BASE}/api/jobs" | jq -r '.[] | "   #\(.id)  \(.stage_name)   [\(.status)]"'

# ---- 4. claim -> run -> report, one job at a time ----
step "4. Claim -> run -> report each job"
while true; do
  job=$(curl -sX POST "${BASE}/api/jobs/claim" \
        -H 'Content-Type: application/json' -d "{\"worker_name\":\"${WORKER}\"}")

  if [ "$job" = "null" ] || [ -z "$job" ]; then
    printf '   %sno more jobs to claim%s\n' "$DIM" "$RST"
    break
  fi

  id=$(echo "$job"    | jq -r '.id')
  stage=$(echo "$job" | jq -r '.stage_name')
  cmd=$(echo "$job"   | jq -r '.command')

  printf '   claimed #%s  %s%s%s   run: %s%s%s\n' "$id" "$YLW" "$stage" "$RST" "$DIM" "$cmd" "$RST"
  sleep 0.4   # pretend the worker is running the command

  curl -sX POST "${BASE}/api/jobs/${id}/report" \
    -H 'Content-Type: application/json' -d '{"status":"passed","output":"ok"}'
  printf '   reported #%s  %spassed%s\n\n' "$id" "$GRN" "$RST"
done

# ---- 5. final status ----
step "5. Final status"
curl -s "${BASE}/api/jobs" | jq -r '.[] | "   #\(.id)  \(.stage_name)   [\(.status)]   worker=\(.assigned_worker // "-")"'

printf '\n%s%sdone%s\n' "$BOLD" "$GRN" "$RST"
