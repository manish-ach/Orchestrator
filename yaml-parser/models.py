from typing import Dict, List
from pydantic import BaseModel, Field


class Job(BaseModel):
    stage: str
    image: str
    script: str
    needs: List[str] = Field(default_factory=list)
    env: Dict[str, str] = Field(default_factory=dict)


class Pipeline(BaseModel):
    name: str
    stages: List[str]
    jobs: Dict[str, Job]