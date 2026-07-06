from parser import load_pipeline
from validator import validate_pipeline
from planner import build_execution_plan


def test_valid_pipeline():
    pipeline = load_pipeline("good.yml")

    validate_pipeline(pipeline)

    plan = build_execution_plan(pipeline)

    assert plan == ["compile", "unit-tests"]