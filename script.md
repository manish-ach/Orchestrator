BASE=http://127.0.0.1:8080     # match your coordinator's --port

# (setup) register a worker so claim can assign to it
```
curl -sX POST $BASE/api/workers/register \
-H 'Content-Type: application/json' -d '{"worker_name":"WorkerA"}'
```

# 1. TRIGGER — create the pipeline's jobs
```
curl -sX POST $BASE/api/pipelines/trigger
```

# 2. QUEUE — see the pending jobs
```
curl -s $BASE/api/jobs | jq
```

# 3. CLAIM — a worker takes the next job (returns the job JSON, or null)
```
curl -sX POST $BASE/api/jobs/claim \
-H 'Content-Type: application/json' -d '{"worker_name":"WorkerA"}' | jq
```

# 4. RUN — (the worker executes the command; no endpoint — this is the worker's job)

# 5. REPORT — tell the coordinator how job #0 ended
```
curl -sX POST $BASE/api/jobs/0/report \
-H 'Content-Type: application/json' -d '{"status":"passed","output":"ok"}'
```

# 6. STATUS — see the updated state
```
curl -s $BASE/api/jobs | jq
```