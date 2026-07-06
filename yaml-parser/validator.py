from graphlib import TopologicalSorter, CycleError
from models import Pipeline


class PipelineValidationError(Exception):
    pass


def validate_pipeline(pipeline: Pipeline):
    validate_stages(pipeline)
    validate_dependencies(pipeline)
    validate_cycles(pipeline)


def validate_stages(pipeline: Pipeline):
    valid_stages = set(pipeline.stages)

    for job_name, job in pipeline.jobs.items():
        if job.stage not in valid_stages:
            raise PipelineValidationError(
                f"Job '{job_name}' uses unknown stage '{job.stage}'"
            )


def validate_dependencies(pipeline: Pipeline):
    all_jobs = set(pipeline.jobs.keys())

    for job_name, job in pipeline.jobs.items():

       
        if job_name in job.needs:
            raise PipelineValidationError(
                f"Job '{job_name}' cannot depend on itself"
            )

        
        for dep in job.needs:
            if dep not in all_jobs:
                raise PipelineValidationError(
                    f"Job '{job_name}' depends on unknown job '{dep}'"
                )


def validate_cycles(pipeline: Pipeline):
    graph = {
        job_name: set(job.needs)
        for job_name, job in pipeline.jobs.items()
    }

    try:
        ts = TopologicalSorter(graph)
        list(ts.static_order())

    except CycleError:
        raise PipelineValidationError(
            "Cyclic dependency detected in pipeline"
        )