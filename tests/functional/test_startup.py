"""The server binary refuses a DATABASE_URL it can't use before connecting,
and says why without printing the URL (invariant 5)."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest

from conftest import SERVER_BIN, Backing

PASSWORD = "startup-canary-pw"
POOLED = (
    f"postgres://app:{PASSWORD}@ep-cool-name-123456-pooler.us-east-2.aws.neon.tech"
    "/neondb?sslmode=verify-full"
)
CHANNEL_BINDING = (
    f"postgres://app:{PASSWORD}@127.0.0.1:1/neondb?sslmode=require&channel_binding=require"
)


def run(command: str, database_url: str, cwd: Path) -> subprocess.CompletedProcess[str]:
    env = {
        "PATH": os.environ["PATH"],
        "HOME": os.environ.get("HOME", "/tmp"),
        "APP_ENV": "development",
        "AUTH_MODE": "dev",
        "BLOB_BACKEND": "memory",
        "BIND_ADDR": "127.0.0.1:0",
        "DATABASE_URL": database_url,
    }
    # From a scratch directory, so the repo's .env can't leak in.
    return subprocess.run(
        [str(SERVER_BIN), command],
        env=env,
        cwd=cwd,
        capture_output=True,
        text=True,
        timeout=30,
    )


def assert_value_free(output: str, url: str) -> None:
    assert url not in output, output
    assert PASSWORD not in output, output
    assert "postgres://" not in output, output


@pytest.mark.parametrize("command", ["migrate", "serve"])
def test_migrations_refuse_the_neon_pooled_url(
    backing: Backing, tmp_path: Path, command: str
) -> None:
    # `serve` migrates first unless RUN_MIGRATIONS=false.
    result = run(command, POOLED, tmp_path)
    output = result.stdout + result.stderr
    assert result.returncode != 0, output
    assert "migrations need a direct connection" in output, output
    # Not even the matched host label: it is text from the URL.
    assert "ep-cool-name" not in output, output
    assert_value_free(output, POOLED)


def test_channel_binding_is_refused_by_name(backing: Backing, tmp_path: Path) -> None:
    result = run("serve", CHANNEL_BINDING, tmp_path)
    output = result.stdout + result.stderr
    assert result.returncode != 0, output
    assert "unsupported query parameters: `channel_binding`" in output, output
    assert_value_free(output, CHANNEL_BINDING)
