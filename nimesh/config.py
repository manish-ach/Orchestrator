# app/config.py

from pathlib import Path
from pydantic_settings import BaseSettings, SettingsConfigDict


BASE_DIR = Path(__file__).resolve().parent.parent


class Settings(BaseSettings):
    # Application
    APP_NAME: str = "Single Node Job Runner"
    APP_VERSION: str = "0.1.0"
    DEBUG: bool = True

    # Database
    DATABASE_URL: str = str(BASE_DIR / "jobs.db")

    # Storage
    LOGS_DIR: str = str(BASE_DIR / "storage" / "logs")

    # Job Execution
    DEFAULT_TIMEOUT: int = 300  # seconds
    MAX_TIMEOUT: int = 3600     # 1 hour

    model_config = SettingsConfigDict(
        env_file=".env",
        env_file_encoding="utf-8",
        extra="ignore",
    )


settings = Settings()

# Ensure directories exist
Path(settings.LOGS_DIR).mkdir(parents=True, exist_ok=True)
#Nimesh Giri