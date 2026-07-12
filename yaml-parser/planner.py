from graphlib import TopologicalSorter
from models import Pipeline


def build_execution_plan(pipeline: Pipeline):
    """
    Return execution order for jobs.
    """

    graph = {
        job_name: set(job.needs)
        for job_name, job in pipeline.jobs.items()
    }

    ts = TopologicalSorter(graph)

    return list(ts.static_order())