import yaml
from models import Pipeline


def load_pipeline(file_path: str) -> Pipeline:
    """
    Load and parse YAML pipeline file.
    """

    with open(file_path, "r") as file:
        data = yaml.safe_load(file)

    return Pipeline.model_validate(data)