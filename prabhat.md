# Module 2: Pipeline Parser & Validator

A small library + CLI that reads a `pipeline.yml` file, validates it, and outputs a structured job graph. Pure logic — no servers, no databases, no Docker. Perfect for an even greener beginner.

## Why this one

- Zero infrastructure to set up
- Pure input → output (easy to test, easy to debug)
- Output plugs directly into the Coordinator API later
- Teaches: file parsing, schema validation, graph data structures, CLI building

## What to build

1. Define a YAML schema for pipelines (stages, jobs, dependencies, env, image)
2. A parser that loads the YAML and turns it into Python objects
3. A validator that catches bad input (missing fields, cyclic dependencies, unknown job refs)
4. A topological sort that produces the execution order
5. A CLI: `pipectl validate pipeline.yml` and `pipectl plan pipeline.yml`

## Example input

```yaml
name: build-and-test
stages:
  - build
  - test
jobs:
  compile:
    stage: build
    image: python:3.11
    script: pip install -e .
  unit-tests:
    stage: test
    image: python:3.11
    script: pytest
    needs: [compile]
```

## Tools

| Purpose | Tool |
|---|---|
| Language | Python 3.11+ |
| YAML parsing | PyYAML |
| Schema validation | Pydantic v2 |
| Graph logic | `graphlib.TopologicalSorter` (stdlib) |
| CLI | Typer or Click |
| Tests | pytest |

## Done when

- `pipectl validate good.yml` exits 0; `pipectl validate broken.yml` prints clear errors and exits 1
- Catches: missing required fields, unknown stages, jobs depending on themselves, cycles
- `pipectl plan pipeline.yml` prints the execution order as a tree or list
- Tests cover ~10 valid and ~10 invalid pipeline examples

## Don't build yet

Running the jobs, talking to workers, webhooks, storage, UI. This module **only** turns YAML into a validated plan.

## How it connects later

Friend #1's job runner executes individual jobs. This module decides **which jobs to run and in what order**. The Coordinator API (a future module) will glue them together.
