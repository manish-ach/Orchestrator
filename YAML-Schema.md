# YAML Validation Checklist

Group validation into layers — run them in order, stop only on fatal errors so users see *all* problems at once.

## 1. File-level safety (do this FIRST)

- Use `yaml.safe_load` — never `yaml.load`. Prevents arbitrary Python object construction.
- Cap file size (e.g., 1 MB). Reject larger files outright.
- Reject files with excessive anchors/aliases (YAML billion-laughs attack).
- UTF-8 only. Reject other encodings.

## 2. Structural (schema) checks

Use Pydantic to enforce shape — most of this is free once the model is defined.

- Required top-level keys: `name`, `jobs`
- Optional top-level: `stages`, `env`, `defaults`
- Field types match (strings are strings, lists are lists)
- **Reject unknown fields** (`extra="forbid"` in Pydantic). Easier to relax later than tighten.
- Job and stage names are unique within their scope

## 3. Naming rules

- Job/stage names match `^[a-z][a-z0-9-]{0,62}$` (DNS-safe, no spaces)
- Reject reserved words: `all`, `none`, `default`, `latest`
- Env var keys match `^[A-Z_][A-Z0-9_]*$`

## 4. Semantic (cross-reference) checks

These need a full pass over the parsed model, not just per-field.

- Every `job.stage` exists in the top-level `stages` list
- Every entry in `job.needs` refers to a real job
- A job cannot list itself in `needs`
- **No cycles** in the `needs` graph — use `graphlib.TopologicalSorter` and catch `CycleError`
- A job in stage N cannot `needs` a job in stage N+1 (only same or earlier stages)
- Warn (don't fail) on orphan jobs nothing depends on, if you adopt a "needs"-driven model

## 5. Resource sanity bounds

Bad values here = runaway costs later. Hard limits matter.

| Field | Min | Max | Default |
|---|---|---|---|
| `timeout` | 1s | 24h | 5m |
| `retries` | 0 | 5 | 0 |
| `parallelism` | 1 | 32 | 1 |
| `script` length | 1 char | 64 KB | — |
| `env` entries per job | 0 | 100 | — |

## 6. Image rules

- `image` is required (no implicit default)
- Must include a tag — reject `python` (no tag) and warn on `python:latest`
- Validate format roughly: `[registry/]repo:tag[@sha256:...]`
- Optionally restrict to an allowlist of registries (later module)

## 7. Script rules

- Must be a non-empty string OR non-empty list of strings
- No shell-injection-friendly patterns from interpolated values (handle at execution time, but flag obvious cases like unquoted `${VAR}` in `rm -rf`)
- Strip and reject if entirely whitespace

## 8. Secret / safety smell-tests (warnings, not errors)

Catch obvious mistakes before they hit a worker:

- Values matching AWS key patterns (`AKIA[0-9A-Z]{16}`)
- Values that look like JWTs, private keys (`-----BEGIN`), bearer tokens
- `env` values longer than ~4 KB (probably a pasted secret)
- Print a warning, don't fail — false positives are common

## 9. Error reporting (this matters as much as the rules)

- **Include line + column numbers** from the YAML node (PyYAML exposes these via `Loader.construct_mapping`)
- **Aggregate errors** — collect all of them, don't bail on the first
- Format: `pipeline.yml:14:5  job 'unit-tests' needs unknown job 'compil'`
- Suggest fixes when cheap (`did you mean 'compile'?` via `difflib.get_close_matches`)

## Validation order summary

```
parse YAML safely
  → schema validation (Pydantic)
    → naming rules
      → cross-reference checks
        → DAG / cycle check
          → resource bounds
            → security smell-tests (warnings)
```

Fail fast only at the parse step. Everything after, collect and report together.
