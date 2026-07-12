import typer
from pydantic import ValidationError

from parser import load_pipeline
from validator import (
    validate_pipeline,
    PipelineValidationError,
)
from planner import build_execution_plan

app = typer.Typer()


def print_summary(pipeline):
    """Print a short overview of the parsed pipeline."""
    typer.secho(f"\nPipeline: {pipeline.name}", fg=typer.colors.CYAN, bold=True)
    typer.echo(f"  Stages ({len(pipeline.stages)}): {', '.join(pipeline.stages)}")
    typer.echo(f"  Jobs   ({len(pipeline.jobs)}):")
    for name, job in pipeline.jobs.items():
        needs = f"  needs: {', '.join(job.needs)}" if job.needs else ""
        typer.echo(f"    - {name}  [stage: {job.stage}]{needs}")


@app.command()
def validate(file: str):
    """
    Validate a pipeline YAML file (schema, stages, dependencies, cycles).
    """

    try:
        typer.echo(f"Reading '{file}' ...")
        pipeline = load_pipeline(file)

        print_summary(pipeline)

        typer.echo("\nRunning checks:")
        typer.secho("  [ok] schema and required fields", fg=typer.colors.GREEN)
        validate_pipeline(pipeline)
        typer.secho("  [ok] stages exist for every job", fg=typer.colors.GREEN)
        typer.secho("  [ok] all dependencies resolve", fg=typer.colors.GREEN)
        typer.secho("  [ok] no dependency cycles", fg=typer.colors.GREEN)

        typer.secho(
            f"\n✓ Pipeline '{pipeline.name}' is valid "
            f"({len(pipeline.jobs)} jobs across {len(pipeline.stages)} stages)",
            fg=typer.colors.GREEN,
            bold=True,
        )

        raise typer.Exit(code=0)

    except ValidationError as e:
        typer.secho("\n✗ Schema validation error:", fg=typer.colors.RED, bold=True)
        typer.echo(e)
        raise typer.Exit(code=1)

    except PipelineValidationError as e:
        typer.secho(f"\n✗ Validation error: {e}", fg=typer.colors.RED, bold=True)
        raise typer.Exit(code=1)

    except FileNotFoundError:
        typer.secho(f"\n✗ File not found: '{file}'", fg=typer.colors.RED, bold=True)
        raise typer.Exit(code=1)


@app.command()
def plan(file: str):
    """
    Validate the pipeline, then print the job execution order.
    """

    try:
        typer.echo(f"Reading '{file}' ...")
        pipeline = load_pipeline(file)
        validate_pipeline(pipeline)

        order = build_execution_plan(pipeline)

        typer.secho(f"\nExecution plan for '{pipeline.name}':", fg=typer.colors.CYAN, bold=True)
        typer.echo(f"({len(order)} jobs, run top to bottom)\n")

        for index, job_name in enumerate(order, start=1):
            job = pipeline.jobs[job_name]
            needs = f"after {', '.join(job.needs)}" if job.needs else "no dependencies"
            typer.secho(f"  {index}. {job_name}", fg=typer.colors.GREEN, bold=True, nl=False)
            typer.echo(f"  [stage: {job.stage}]  ({needs})")

        typer.secho("\n✓ Plan generated successfully", fg=typer.colors.GREEN)

    except ValidationError as e:
        typer.secho("\n✗ Schema validation error:", fg=typer.colors.RED, bold=True)
        typer.echo(e)
        raise typer.Exit(code=1)

    except PipelineValidationError as e:
        typer.secho(f"\n✗ Validation error: {e}", fg=typer.colors.RED, bold=True)
        raise typer.Exit(code=1)

    except FileNotFoundError:
        typer.secho(f"\n✗ File not found: '{file}'", fg=typer.colors.RED, bold=True)
        raise typer.Exit(code=1)


if __name__ == "__main__":
    app()
