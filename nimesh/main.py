# app/main.py

from contextlib import asynccontextmanager

from fastapi import FastAPI

from app.api.routes.jobs import router as jobs_router
from app.db.session import init_db


@asynccontextmanager
async def lifespan(app: FastAPI):
    # Initialize database on startup
    init_db()

    # Future: recover running jobs after restart
    yield

    # Cleanup logic can go here


app = FastAPI(
    title="Single Node Job Runner",
    version="0.1.0",
    lifespan=lifespan,
)


@app.get("/", tags=["Health"])
async def health_check():
    return {
        "status": "ok",
        "service": "job-runner",
    }


@app.get("/health", tags=["Health"])
async def health():
    return {"healthy": True}


# Register routes
app.include_router(
    jobs_router,
    prefix="/jobs",
    tags=["Jobs"],
)
#Nimesh Giri