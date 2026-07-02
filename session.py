# app/db/session.py

import sqlite3
from contextlib import contextmanager

from app.config import settings


def get_connection():
    """Create a new SQLite connection."""
    conn = sqlite3.connect(settings.DATABASE_URL)
    conn.row_factory = sqlite3.Row
    return conn


def init_db():
    """Initialize the database and create tables if they don't exist."""
    with get_connection() as conn:
        cursor = conn.cursor()

        cursor.execute("""
        CREATE TABLE IF NOT EXISTS jobs (
            id TEXT PRIMARY KEY,
            command TEXT NOT NULL,
            env TEXT,
            timeout INTEGER NOT NULL,
            status TEXT NOT NULL,
            exit_code INTEGER,
            stdout_path TEXT,
            stderr_path TEXT,
            pid INTEGER,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            started_at TIMESTAMP,
            finished_at TIMESTAMP
        )
        """)

        conn.commit()


@contextmanager
def get_db():
    """
    Context manager for database operations.

    Usage:
        with get_db() as conn:
            cursor = conn.cursor()
            cursor.execute(...)
    """
    conn = get_connection()
    try:
        yield conn
        conn.commit()
    except Exception:
        conn.rollback()
        raise
    finally:
        conn.close()