"""Machine-readable entry point for the coordinator.

    python plan_json.py <pipeline.yml>

Validates the pipeline (schema, stages, dependencies, cycles) and prints the
execution plan as JSON on stdout:

    {
      "name": "demo-app",
      "stages": ["build", "test", "deploy"],
      "jobs": [                                 # in execution order
        {"name": "compile", "stage": "build", "command": "cargo build",
         "needs": [], "env": {}, "image": "rust:latest"},
        ...
      ]
    }

On any error it prints {"error": "<reason>"} and exits 1, so the caller can
surface the message directly.
"""

import json
import sys

from pydantic import ValidationError

from parser import load_pipeline
from planner import build_execution_plan
from validator import PipelineValidationError, validate_pipeline


def plan_json(file_path: str) -> dict:
    pipeline = load_pipeline(file_path)
    validate_pipeline(pipeline)
    order = build_execution_plan(pipeline)

    return {
        "name": pipeline.name,
        "stages": pipeline.stages,
        "jobs": [
            {
                "name": job_name,
                "stage": pipeline.jobs[job_name].stage,
                "command": pipeline.jobs[job_name].script,
                "needs": pipeline.jobs[job_name].needs,
                "env": pipeline.jobs[job_name].env,
                "image": pipeline.jobs[job_name].image,
                "artifacts": pipeline.jobs[job_name].artifacts,
                "worker": pipeline.jobs[job_name].worker,
                "tags": pipeline.jobs[job_name].tags,
            }
            for job_name in order
        ],
    }


def main() -> int:
    if len(sys.argv) != 2:
        print(json.dumps({"error": "usage: plan_json.py <pipeline.yml>"}))
        return 1

    try:
        print(json.dumps(plan_json(sys.argv[1])))
        return 0
    except FileNotFoundError:
        print(json.dumps({"error": f"file not found: {sys.argv[1]}"}))
    except ValidationError as e:
        first = e.errors()[0]
        loc = ".".join(str(p) for p in first.get("loc", ()))
        print(json.dumps({"error": f"schema error at '{loc}': {first.get('msg', 'invalid')}"}))
    except PipelineValidationError as e:
        print(json.dumps({"error": str(e)}))
    except Exception as e:  # yaml syntax errors etc.
        print(json.dumps({"error": f"{type(e).__name__}: {e}"}))
    return 1


if __name__ == "__main__":
    sys.exit(main())
