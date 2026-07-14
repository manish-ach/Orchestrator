from typing import Dict, List, Optional
from pydantic import BaseModel, Field


class Job(BaseModel):
    stage: str
    image: str
    script: str
    needs: List[str] = Field(default_factory=list)
    env: Dict[str, str] = Field(default_factory=dict)
    # workspace paths uploaded when the job passes; downloaded by jobs
    # that `needs` this one (possibly on another machine)
    artifacts: List[str] = Field(default_factory=list)
    # pin the job to one worker by name (e.g. the machine with the docker
    # socket); sugar for env WORKER_PIN
    worker: Optional[str] = None
    # capability labels — the coordinator only places the job on a worker
    # registered with all of them (e.g. tags: [heavy])
    tags: List[str] = Field(default_factory=list)


class Pipeline(BaseModel):
    name: str
    stages: List[str]
    jobs: Dict[str, Job]