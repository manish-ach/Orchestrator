import pytest

from parser import load_pipeline
from validator import (
    validate_pipeline,
    PipelineValidationError,
)


def test_invalid_stage():
    pipeline = load_pipeline("bad_stage.yml")

    with pytest.raises(PipelineValidationError):
        validate_pipeline(pipeline)


def test_cycle_detection():
    pipeline = load_pipeline("bad_cycle.yml")

    with pytest.raises(PipelineValidationError):
        validate_pipeline(pipeline)


def test_self_dependency():
    pipeline = load_pipeline("bad_self_dep.yml")

    with pytest.raises(PipelineValidationError):
        validate_pipeline(pipeline)