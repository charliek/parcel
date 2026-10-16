"""Functional-test harness: the real binaries, end to end.

One session-scoped ``parcel-server`` process runs against:

* the compose Postgres (database ``parcel_functional``, recreated per session),
* the compose GCS emulator (a fresh bucket per session) — so the production
  blob-store code path is what these tests exercise,
* a tiny in-process JWKS server whose RSA key the tests use to mint slauth-
  shaped access tokens.

The app origin is ``http://localhost:<port>`` and the content origin is
``http://127.0.0.1:<port>``: the same listener, told apart by ``Host``, which
is exactly how production splits the two origins.

Nothing is skipped when a dependency is missing: the suite fails loudly with
the command that fixes it (``docker compose up -d``).
"""

from __future__ import annotations

import base64
import json
import os
import socket
import subprocess
import threading
import time
import uuid
from collections.abc import Iterator
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import httpx
import jwt
import pytest
from cryptography.hazmat.primitives.asymmetric import rsa

ROOT = Path(__file__).resolve().parents[2]
TARGET = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")) / "debug"
SERVER_BIN = TARGET / "parcel-server"
CLI_BIN = TARGET / "parcel"
PG_ADMIN_URL = os.environ.get(
    "PARCEL_FUNCTIONAL_PG_ADMIN", "postgres://postgres:localdev@localhost:5439/postgres"
)
DB_NAME = "parcel_functional"
GCS_EMULATOR = os.environ.get("PARCEL_FUNCTIONAL_GCS", "http://localhost:4447")
ISSUER = "https://auth.functional.test"
KID = "functional-test-key"


def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _b64url_uint(value: int) -> str:
    raw = value.to_bytes((value.bit_length() + 7) // 8, "big")
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


@dataclass
class Issuer:
    """Mints slauth-shaped RS256 access tokens trusted by the server."""

    key: rsa.RSAPrivateKey
    jwks_url: str

    def token(self, sub: str | None = None, email: str | None = None, **extra) -> str:
        sub = sub or str(uuid.uuid4())
        now = int(time.time())
        claims = {
            "iss": ISSUER,
            "aud": ["parcel"],
            "sub": sub,
            "email": email or f"{sub}@example.com",
            "name": "Functional Tester",
            "iat": now,
            "exp": now + 900,
            **extra,
        }
        return jwt.encode(claims, self.key, algorithm="RS256", headers={"kid": KID})


@pytest.fixture(scope="session")
def issuer() -> Iterator[Issuer]:
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    numbers = key.public_key().public_numbers()
    jwks = json.dumps(
        {
            "keys": [
                {
                    "kty": "RSA",
                    "use": "sig",
                    "alg": "RS256",
                    "kid": KID,
                    "n": _b64url_uint(numbers.n),
                    "e": _b64url_uint(numbers.e),
                }
            ]
        }
    ).encode()

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):  # noqa: N802 (http.server naming)
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(jwks)))
            self.end_headers()
            self.wfile.write(jwks)

        def log_message(self, *_args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    yield Issuer(key=key, jwks_url=f"http://127.0.0.1:{server.server_port}/jwks.json")
    server.shutdown()


def _recreate_database() -> str:
    """Drop and recreate parcel_functional via psql inside the compose container."""
    sql = f"DROP DATABASE IF EXISTS {DB_NAME} WITH (FORCE); CREATE DATABASE {DB_NAME};"
    result = subprocess.run(
        ["docker", "exec", "-i", "parcel-postgres", "psql", "-U", "postgres", "-v", "ON_ERROR_STOP=1"],
        input=sql.replace("; ", ";\n"),
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        pytest.fail(
            "could not reset the functional database — is the compose Postgres up?\n"
            f"  run: docker compose up -d\n{result.stderr}",
            pytrace=False,
        )
    return PG_ADMIN_URL.rsplit("/", 1)[0] + f"/{DB_NAME}"


def _create_bucket() -> str:
    name = f"parcel-functional-{uuid.uuid4().hex[:12]}"
    try:
        response = httpx.post(
            f"{GCS_EMULATOR}/storage/v1/b?project=functional", json={"name": name}, timeout=5
        )
    except httpx.HTTPError as e:
        pytest.fail(
            f"fake-gcs-server is not reachable at {GCS_EMULATOR} ({e}).\n"
            "  run: docker compose up -d fake-gcs",
            pytrace=False,
        )
    assert response.status_code in (200, 201), response.text
    return name


def _build_binaries() -> None:
    if os.environ.get("PARCEL_FUNCTIONAL_SKIP_BUILD"):
        return
    subprocess.run(
        ["mise", "exec", "--", "cargo", "build", "-p", "parcel-server", "-p", "parcel-cli"],
        cwd=ROOT,
        check=True,
    )


@dataclass
class Server:
    app_url: str
    content_url: str
    issuer: Issuer
    bucket: str
    log_path: Path

    def client(self, token: str | None = None) -> httpx.Client:
        headers = {"Authorization": f"Bearer {token}"} if token else {}
        return httpx.Client(base_url=self.app_url, headers=headers, timeout=30)

    def content(self) -> httpx.Client:
        """A client for the content origin (no credentials, like a browser frame)."""
        return httpx.Client(timeout=30, follow_redirects=False)


LIMITS = {
    # Small enough that limit tests stay fast, large enough for real pages.
    "PARCEL_MAX_ENTRY_BYTES": str(2 * 1024 * 1024),
    "PARCEL_MAX_FILE_BYTES": str(1024 * 1024),
    "PARCEL_MAX_UPLOAD_BYTES": str(4 * 1024 * 1024),
    "PARCEL_MAX_FILES_PER_UPLOAD": "40",
}


@pytest.fixture(scope="session")
def server(issuer: Issuer, tmp_path_factory: pytest.TempPathFactory) -> Iterator[Server]:
    _build_binaries()
    database_url = _recreate_database()
    bucket = _create_bucket()
    port = _free_port()
    log_path = tmp_path_factory.mktemp("server") / "parcel-server.log"
    env = {
        "PATH": os.environ["PATH"],
        "HOME": os.environ.get("HOME", "/tmp"),
        "APP_ENV": "development",
        "RUST_LOG": "info,parcel_server=debug",
        "BIND_ADDR": f"127.0.0.1:{port}",
        "DATABASE_URL": database_url,
        "PARCEL_APP_ORIGIN": f"http://localhost:{port}",
        "PARCEL_CONTENT_ORIGIN": f"http://127.0.0.1:{port}",
        "PARCEL_CONTENT_SIGNING_KEY": "functional-test-signing-key-0123456789abcdef",
        "AUTH_MODE": "slauth",
        "SLAUTH_ISSUER": ISSUER,
        "SLAUTH_JWKS_URL": issuer.jwks_url,
        "SLAUTH_AUDIENCE": "parcel",
        "BLOB_BACKEND": "gcs",
        "GCS_BUCKET": bucket,
        "GCS_PREFIX": "functional",
        "STORAGE_EMULATOR_HOST": GCS_EMULATOR,
        **LIMITS,
    }
    with open(log_path, "w") as log:
        # Run from a scratch directory so the repo's .env can't leak in.
        process = subprocess.Popen(
            [str(SERVER_BIN), "serve"],
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
            cwd=log_path.parent,
        )
    app_url = f"http://localhost:{port}"
    deadline = time.time() + 30
    while True:
        if process.poll() is not None:
            pytest.fail(f"parcel-server exited early:\n{log_path.read_text()}", pytrace=False)
        try:
            if httpx.get(f"{app_url}/health/ready", timeout=1).status_code == 200:
                break
        except httpx.HTTPError:
            pass
        if time.time() > deadline:
            process.kill()
            pytest.fail(f"parcel-server never became ready:\n{log_path.read_text()}", pytrace=False)
        time.sleep(0.2)
    yield Server(
        app_url=app_url,
        content_url=f"http://127.0.0.1:{port}",
        issuer=issuer,
        bucket=bucket,
        log_path=log_path,
    )
    process.terminate()
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()


@pytest.fixture
def user(server: Server) -> str:
    """A fresh user's access token."""
    return server.issuer.token()


@pytest.fixture
def api(server: Server, user: str) -> Iterator[httpx.Client]:
    with server.client(user) as client:
        yield client


@dataclass
class Cli:
    """Runs the `parcel` binary with an isolated HOME and config dir."""

    server: Server
    home: Path

    def env(self, token: str | None) -> dict[str, str]:
        env = {
            "PATH": os.environ["PATH"],
            "HOME": str(self.home),
            "XDG_CONFIG_HOME": str(self.home / ".config"),
            "PARCEL_CONFIG_DIR": str(self.home / ".config" / "parcel"),
            "PARCEL_SERVER": self.server.app_url,
            "NO_COLOR": "1",
        }
        if token:
            env["PARCEL_TOKEN"] = token
        return env

    def run(
        self,
        *args: str,
        token: str | None = None,
        input: str | None = None,
        cwd: Path | None = None,
        check: bool = True,
    ) -> subprocess.CompletedProcess[str]:
        result = subprocess.run(
            [str(CLI_BIN), *args],
            env=self.env(token),
            input=input,
            capture_output=True,
            text=True,
            cwd=cwd or self.home,
            timeout=60,
            check=False,
        )
        if check and result.returncode != 0:
            raise AssertionError(
                f"parcel {' '.join(args)} exited {result.returncode}\n"
                f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
            )
        return result

    def json(self, *args: str, token: str | None = None):
        return json.loads(self.run(*args, "--json", token=token).stdout)


@pytest.fixture
def cli(server: Server, tmp_path: Path) -> Cli:
    home = tmp_path / "home"
    home.mkdir()
    return Cli(server=server, home=home)
