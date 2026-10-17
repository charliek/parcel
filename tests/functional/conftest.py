"""Functional-test harness: the real binaries, end to end.

One session-scoped ``parcel-server`` process runs against:

* the compose Postgres (database ``parcel_functional``, recreated per session),
* the compose GCS emulator (a fresh bucket per session) — so the production
  blob-store code path is what these tests exercise,
* a small in-process OpenID provider (:class:`MockIdp`) serving two tenants,
  configured as two sign-in providers: ``test`` (a public client, any email
  domain) and ``corp`` (a confidential client limited to ``corp.example``,
  with provider logout).

People sign in the way a browser does: ``/auth/login`` → the provider's
``/authorize`` → ``/auth/callback``, redirects followed by hand (see
:func:`sign_in`). The CLI uses ``pcl_`` tokens, as it does for real.

The app origin is ``http://localhost:<port>`` and the content origin is
``http://127.0.0.1:<port>``: the same listener, told apart by ``Host``, which
is exactly how production splits the two origins.

Nothing is skipped when a dependency is missing: the suite fails loudly with
the command that fixes it (``docker compose up -d``).
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import secrets
import shutil
import socket
import subprocess
import threading
import time
import uuid
from collections.abc import Callable, Iterator
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlencode, urlsplit

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
PG_CONTAINER = "parcel-postgres"
DB_NAME = "parcel_functional"
GCS_EMULATOR = os.environ.get("PARCEL_FUNCTIONAL_GCS", "http://localhost:4447")
KID = "functional-oidc-key"
SIGNING_KEY = "functional-test-signing-key-0123456789abcdef"
TOKEN_LOGIN_MAX_AGE_DAYS = 30


def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _b64url(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def _b64url_uint(value: int) -> str:
    return _b64url(value.to_bytes((value.bit_length() + 7) // 8, "big"))


# ---------------------------------------------------------------- mock OpenID provider


@dataclass
class Tenant:
    """One provider the server is configured with (``AUTH_<ID>_*``)."""

    id: str
    label: str
    client_id: str
    client_secret: str | None = None
    allowed_domains: str | None = None
    provider_logout: bool = False
    issuer: str = ""

    def env(self) -> dict[str, str]:
        prefix = f"AUTH_{self.id.upper()}_"
        env = {
            f"{prefix}ISSUER": self.issuer,
            f"{prefix}CLIENT_ID": self.client_id,
            f"{prefix}LABEL": self.label,
        }
        if self.client_secret:
            env[f"{prefix}CLIENT_SECRET"] = self.client_secret
        if self.allowed_domains:
            env[f"{prefix}ALLOWED_DOMAINS"] = self.allowed_domains
        if self.provider_logout:
            env[f"{prefix}PROVIDER_LOGOUT"] = "true"
        return env


class MockIdp:
    """A small OpenID provider: discovery, ``/authorize``, ``/token``,
    ``/jwks.json``, ``/userinfo`` and an ``end_session`` endpoint per tenant,
    all under ``http://127.0.0.1:<port>/<tenant>``.

    ``/authorize`` signs in whoever ``login_hint`` names (no login page), with
    what :meth:`profile` set for them; ``/token`` checks the client, the
    redirect URI and the PKCE verifier, and returns an RS256 ID token. Every
    request is recorded so tests can check what the server sent.
    """

    def __init__(self, tenants: list[Tenant]) -> None:
        self.key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
        self.tenants = {t.id: t for t in tenants}
        self._profiles: dict[tuple[str, str], dict] = {}
        self._codes: dict[str, dict] = {}
        self._access_tokens: dict[str, dict] = {}
        self.authorize_requests: list[dict[str, str]] = []
        self.token_requests: list[dict] = []
        self.token_errors: list[str] = []
        self._lock = threading.Lock()
        self._http = ThreadingHTTPServer(("127.0.0.1", 0), self._handler())
        self.base = f"http://127.0.0.1:{self._http.server_port}"
        for tenant in tenants:
            tenant.issuer = f"{self.base}/{tenant.id}"
        threading.Thread(target=self._http.serve_forever, daemon=True).start()

    def close(self) -> None:
        self._http.shutdown()

    # -- what tests control

    def profile(self, email: str, /, provider: str = "test", **claims) -> None:
        """How ``email`` signs in at ``provider`` from now on. Claims override
        the defaults (``sub``, ``email_verified: true``, a ``name``); special
        keys: ``deny=True`` refuses the sign-in (``access_denied``), and
        ``email_in_id_token=False`` leaves identity claims to ``userinfo``."""
        with self._lock:
            self._profiles.setdefault((provider, email.lower()), {}).update(claims)

    def id_token(self, provider: str = "test", **claims) -> str:
        """An ID token this provider would issue (for "bearer JWTs are refused")."""
        tenant = self.tenants[provider]
        now = int(time.time())
        body = {"iss": tenant.issuer, "aud": tenant.client_id, "iat": now, "exp": now + 300}
        body.update(claims)
        return jwt.encode(body, self.key, algorithm="RS256", headers={"kid": KID})

    def last_authorize(self, provider: str) -> dict[str, str]:
        return next(r for r in reversed(self.authorize_requests) if r["_tenant"] == provider)

    # -- the provider

    def _identity(self, provider: str, email: str) -> dict:
        local = email.split("@", 1)[0]
        defaults = {
            "sub": f"{provider}-{hashlib.sha256(email.encode()).hexdigest()[:20]}",
            "email": email,
            "email_verified": True,
            "name": local.replace(".", " ").replace("-", " ").title(),
        }
        with self._lock:
            return {**defaults, **self._profiles.get((provider, email.lower()), {})}

    def _discovery(self, tenant: Tenant) -> dict:
        issuer = tenant.issuer
        methods = ["client_secret_basic", "client_secret_post"] if tenant.client_secret else ["none"]
        return {
            "issuer": issuer,
            "authorization_endpoint": f"{issuer}/authorize",
            "token_endpoint": f"{issuer}/token",
            "jwks_uri": f"{issuer}/jwks.json",
            "userinfo_endpoint": f"{issuer}/userinfo",
            "end_session_endpoint": f"{issuer}/logout",
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": methods,
        }

    def _jwks(self) -> dict:
        numbers = self.key.public_key().public_numbers()
        return {
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

    def _authorize(self, tenant: Tenant, q: dict[str, str]) -> tuple[int, dict, str]:
        self.authorize_requests.append({**q, "_tenant": tenant.id})
        if q.get("client_id") != tenant.client_id or not q.get("redirect_uri"):
            return 400, {}, "unknown client or no redirect_uri"
        problems = [
            q.get("response_type") != "code",
            "openid" not in q.get("scope", "").split(),
            q.get("code_challenge_method") != "S256",
            not q.get("code_challenge"),
            not q.get("state"),
            not q.get("nonce"),
            not q.get("login_hint"),  # the mock has no login page
        ]
        redirect = q["redirect_uri"]
        if any(problems):
            return 302, {"Location": f"{redirect}?{urlencode({'error': 'invalid_request', 'state': q.get('state', '')})}"}, ""
        identity = self._identity(tenant.id, q["login_hint"])
        if identity.pop("deny", False):
            return 302, {"Location": f"{redirect}?{urlencode({'error': 'access_denied', 'state': q['state']})}"}, ""
        code = secrets.token_urlsafe(24)
        with self._lock:
            self._codes[code] = {
                "tenant": tenant.id,
                "redirect_uri": redirect,
                "nonce": q["nonce"],
                "challenge": q["code_challenge"],
                "identity": identity,
            }
        return 302, {"Location": f"{redirect}?{urlencode({'code': code, 'state': q['state']})}"}, ""

    def _token(self, tenant: Tenant, form: dict[str, str], authorization: str | None) -> tuple[int, dict]:
        self.token_requests.append({**form, "_tenant": tenant.id, "_authorization": authorization})

        def refuse(error: str, why: str) -> tuple[int, dict]:
            self.token_errors.append(why)
            return 400, {"error": error, "error_description": why}

        if tenant.client_secret:
            expected = "Basic " + base64.b64encode(
                f"{tenant.client_id}:{tenant.client_secret}".encode()
            ).decode()
            if authorization != expected:
                return refuse("invalid_client", "confidential client without Basic auth")
        elif form.get("client_id") != tenant.client_id or "client_secret" in form:
            return refuse("invalid_client", "public client must send its client_id only")
        if form.get("grant_type") != "authorization_code":
            return refuse("unsupported_grant_type", form.get("grant_type", ""))
        with self._lock:
            grant = self._codes.pop(form.get("code", ""), None)  # single use
        if grant is None or grant["tenant"] != tenant.id:
            return refuse("invalid_grant", "unknown or reused code")
        if form.get("redirect_uri") != grant["redirect_uri"]:
            return refuse("invalid_grant", "redirect_uri differs from /authorize")
        verifier = form.get("code_verifier", "")
        if _b64url(hashlib.sha256(verifier.encode()).digest()) != grant["challenge"]:
            return refuse("invalid_grant", "PKCE verifier does not match the challenge")

        identity = dict(grant["identity"])
        in_id_token = identity.pop("email_in_id_token", True)
        claims = {"sub": identity["sub"], "nonce": grant["nonce"]}
        if in_id_token:
            claims.update({k: v for k, v in identity.items() if k != "sub"})
        access_token = secrets.token_urlsafe(24)
        with self._lock:
            self._access_tokens[access_token] = identity
        return 200, {
            "access_token": access_token,
            "token_type": "Bearer",
            "expires_in": 300,
            "id_token": self.id_token(tenant.id, **claims),
        }

    def _userinfo(self, authorization: str | None) -> tuple[int, dict]:
        token = (authorization or "").removeprefix("Bearer ")
        with self._lock:
            identity = self._access_tokens.get(token)
        if identity is None:
            return 401, {"error": "invalid_token"}
        return 200, identity

    def _handler(self):
        idp = self

        class Handler(BaseHTTPRequestHandler):
            def _send(self, status: int, body: dict | str = "", headers: dict | None = None):
                raw = (json.dumps(body) if isinstance(body, dict) else body).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Cache-Control", "no-store")
                for k, v in (headers or {}).items():
                    self.send_header(k, v)
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)

            def _route(self) -> tuple[Tenant | None, str, dict[str, str]]:
                parts = urlsplit(self.path)
                tenant_id, _, rest = parts.path.lstrip("/").partition("/")
                query = {k: v[0] for k, v in parse_qs(parts.query).items()}
                return idp.tenants.get(tenant_id), rest, query

            def do_GET(self):  # noqa: N802 (http.server naming)
                tenant, rest, query = self._route()
                if tenant is None:
                    return self._send(404, {"error": "no such tenant"})
                if rest == ".well-known/openid-configuration":
                    return self._send(200, idp._discovery(tenant))
                if rest == "jwks.json":
                    return self._send(200, idp._jwks())
                if rest == "authorize":
                    status, headers, text = idp._authorize(tenant, query)
                    return self._send(status, text, headers)
                if rest == "userinfo":
                    return self._send(*idp._userinfo(self.headers.get("Authorization")))
                return self._send(404, {"error": "not found"})

            def do_POST(self):  # noqa: N802
                tenant, rest, _ = self._route()
                length = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(length).decode()
                if tenant is None or rest != "token":
                    return self._send(404, {"error": "not found"})
                if not self.headers.get("Content-Type", "").startswith(
                    "application/x-www-form-urlencoded"
                ):
                    return self._send(400, {"error": "invalid_request"})
                form = {k: v[0] for k, v in parse_qs(body).items()}
                return self._send(*idp._token(tenant, form, self.headers.get("Authorization")))

            def log_message(self, *_args):
                pass

        return Handler


TENANTS = [
    Tenant(id="test", label="Test IdP", client_id="parcel-functional"),
    Tenant(
        id="corp",
        label="Corp SSO",
        client_id="parcel-corp",
        client_secret="corp-secret",
        allowed_domains="corp.example",
        provider_logout=True,
    ),
]


@pytest.fixture(scope="session")
def idp() -> Iterator[MockIdp]:
    provider = MockIdp(TENANTS)
    yield provider
    provider.close()


# ---------------------------------------------------------------- the server


def _psql(sql: str, database: str = "postgres") -> str:
    result = subprocess.run(
        [
            "docker", "exec", "-i", PG_CONTAINER,
            "psql", "-U", "postgres", "-d", database, "-v", "ON_ERROR_STOP=1", "-At",
        ],
        input=sql,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        pytest.fail(
            "psql failed — is the compose Postgres up?\n"
            f"  run: docker compose up -d\n{result.stderr}",
            pytrace=False,
        )
    return result.stdout


def _recreate_database() -> str:
    """Drop and recreate parcel_functional via psql inside the compose container."""
    _psql(f"DROP DATABASE IF EXISTS {DB_NAME} WITH (FORCE);\nCREATE DATABASE {DB_NAME};\n")
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
    # mise pins the toolchain locally; CI puts the pinned cargo on PATH itself.
    cargo = ["mise", "exec", "--", "cargo"] if shutil.which("mise") else ["cargo"]
    subprocess.run(
        [*cargo, "build", "-p", "parcel-server", "-p", "parcel-cli"],
        cwd=ROOT,
        check=True,
    )


class Browser(httpx.Client):
    """A signed-in person's browser on the app origin: it keeps the session
    cookie, and like a browser it sends ``Origin`` on every non-GET request
    (cookie-authenticated writes need it)."""

    def __init__(self, server: Server, email: str) -> None:
        def add_origin(request: httpx.Request) -> None:
            if request.method not in ("GET", "HEAD", "OPTIONS"):
                request.headers.setdefault("Origin", server.app_url)

        super().__init__(
            base_url=server.app_url,
            timeout=30,
            follow_redirects=False,
            event_hooks={"request": [add_origin]},
        )
        self.server = server
        self.email = email
        self.landed: str | None = None

    def token(self, name: str = "functional") -> str:
        """Mint a ``pcl_`` API token for this person, as the web Tokens page does."""
        created = self.post("/api/v1/tokens", json={"name": name})
        assert created.status_code == 201, created.text
        return created.json()["secret"]

    def me(self) -> httpx.Response:
        return self.get("/api/v1/me")


@dataclass
class Server:
    app_url: str
    content_url: str
    idp: MockIdp
    bucket: str
    log_path: Path
    process: subprocess.Popen = field(repr=False)

    def client(self, token: str | None = None) -> httpx.Client:
        """A plain API client: a ``pcl_`` bearer token, or anonymous."""
        headers = {"Authorization": f"Bearer {token}"} if token else {}
        return httpx.Client(base_url=self.app_url, headers=headers, timeout=30)

    def content(self) -> httpx.Client:
        """A client for the content origin (no credentials, like a browser frame)."""
        return httpx.Client(timeout=30, follow_redirects=False)

    def sql(self, statement: str) -> str:
        """Run SQL against the server's database (for what has no API yet)."""
        return _psql(statement, DB_NAME)

    def begin_sign_in(
        self,
        browser: Browser,
        email: str | None,
        provider: str = "test",
        return_to: str = "/",
    ) -> httpx.Response:
        """``/auth/login`` → the provider → ``/auth/callback``, by hand.
        Returns the server's last answer: the callback's redirect, or the
        login's when it never reached the provider."""
        params = {"provider": provider, "return_to": return_to}
        if email:
            params["login_hint"] = email
        login = browser.get("/auth/login", params=params)
        assert login.status_code == 303, login.text
        authorize = login.headers["location"]
        if not authorize.startswith(self.idp.tenants[provider].issuer):
            return login
        at_provider = httpx.get(authorize, timeout=30)  # its own origin: no parcel cookies
        assert at_provider.status_code == 302, at_provider.text
        callback = at_provider.headers["location"]
        assert callback.startswith(f"{self.app_url}/auth/callback?"), callback
        answer = browser.get(callback)
        assert answer.status_code == 303, answer.text
        return answer

    def sign_in(
        self,
        email: str,
        /,
        provider: str = "test",
        return_to: str = "/",
        browser: Browser | None = None,
        **profile,
    ) -> Browser:
        """Sign ``email`` in through ``provider`` and return the browser
        holding its session. ``profile`` claims go to the provider first."""
        if profile:
            self.idp.profile(email, provider, **profile)
        browser = browser or Browser(self, email)
        answer = self.begin_sign_in(browser, email, provider, return_to)
        landed = answer.headers["location"]
        assert landed == return_to, f"sign-in of {email} via {provider} landed on {landed}"
        browser.landed = landed
        return browser


def signin_error(response: httpx.Response) -> str | None:
    """The ``error`` code of a redirect to ``/signin``, or None."""
    location = urlsplit(response.headers.get("location", ""))
    if location.path != "/signin":
        return None
    return parse_qs(location.query).get("error", [None])[0]


# Page data limits, small so the 413 paths are cheap to reach.
MAX_DATA_VALUE_BYTES = 8 * 1024
MAX_DATA_BYTES = 32 * 1024

LIMITS = {
    # Small enough that limit tests stay fast, large enough for real pages.
    "PARCEL_MAX_ENTRY_BYTES": str(2 * 1024 * 1024),
    "PARCEL_MAX_FILE_BYTES": str(1024 * 1024),
    "PARCEL_MAX_UPLOAD_BYTES": str(4 * 1024 * 1024),
    "PARCEL_MAX_FILES_PER_UPLOAD": "40",
    "PARCEL_MAX_DATA_VALUE_BYTES": str(MAX_DATA_VALUE_BYTES),
    "PARCEL_MAX_DATA_BYTES": str(MAX_DATA_BYTES),
}


def _start_server(
    idp: MockIdp, database_url: str, bucket: str, log_dir: Path, extra: dict[str, str]
) -> Server:
    port = _free_port()
    log_path = log_dir / "parcel-server.log"
    env = {
        "PATH": os.environ["PATH"],
        "HOME": os.environ.get("HOME", "/tmp"),
        "APP_ENV": "development",  # allows the provider's http:// issuer
        "RUST_LOG": "info,parcel_server=debug",
        "BIND_ADDR": f"127.0.0.1:{port}",
        "DATABASE_URL": database_url,
        "PARCEL_APP_ORIGIN": f"http://localhost:{port}",
        "PARCEL_CONTENT_ORIGIN": f"http://127.0.0.1:{port}",
        "PARCEL_CONTENT_SIGNING_KEY": SIGNING_KEY,
        "AUTH_MODE": "oidc",
        "AUTH_PROVIDERS": ",".join(t.id for t in idp.tenants.values()),
        "PARCEL_TOKEN_LOGIN_MAX_AGE_DAYS": str(TOKEN_LOGIN_MAX_AGE_DAYS),
        "BLOB_BACKEND": "gcs",
        "GCS_BUCKET": bucket,
        "GCS_PREFIX": "functional",
        "STORAGE_EMULATOR_HOST": GCS_EMULATOR,
        **LIMITS,
        **{k: v for t in idp.tenants.values() for k, v in t.env().items()},
        **extra,
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
    return Server(
        app_url=app_url,
        content_url=f"http://127.0.0.1:{port}",
        idp=idp,
        bucket=bucket,
        log_path=log_path,
        process=process,
    )


def _stop(server: Server) -> None:
    server.process.terminate()
    try:
        server.process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        server.process.kill()


@dataclass
class Backing:
    database_url: str
    bucket: str


@pytest.fixture(scope="session")
def backing() -> Backing:
    _build_binaries()
    return Backing(database_url=_recreate_database(), bucket=_create_bucket())


@pytest.fixture(scope="session")
def server(
    idp: MockIdp, backing: Backing, tmp_path_factory: pytest.TempPathFactory
) -> Iterator[Server]:
    running = _start_server(
        idp, backing.database_url, backing.bucket, tmp_path_factory.mktemp("server"), {}
    )
    yield running
    _stop(running)


@pytest.fixture(scope="session")
def no_public_server(
    server: Server, idp: MockIdp, backing: Backing, tmp_path_factory: pytest.TempPathFactory
) -> Iterator[Server]:
    """A second server with ``PARCEL_ALLOW_PUBLIC=false`` on the same database
    and bucket, so a parcel made public through ``server`` shows how this one
    treats it."""
    running = _start_server(
        idp,
        backing.database_url,
        backing.bucket,
        tmp_path_factory.mktemp("no-public-server"),
        {"PARCEL_ALLOW_PUBLIC": "false"},
    )
    yield running
    _stop(running)


def unique_email(name: str, domain: str = "example.com") -> str:
    slug = "-".join(name.lower().split()) or "person"
    return f"{slug}-{uuid.uuid4().hex[:8]}@{domain}"


@pytest.fixture
def people(server: Server) -> Iterator[Callable[..., Browser]]:
    """Sign a new person in: ``people("Ada Lovelace")`` → their browser.
    Takes ``email=``, ``provider=`` and provider claims (``name=`` defaults
    to the one given)."""
    opened: list[Browser] = []

    def sign_in(name: str = "Functional Tester", email: str | None = None, **kw) -> Browser:
        email = email or unique_email(name)
        kw.setdefault("name", name)
        browser = server.sign_in(email, **kw)
        opened.append(browser)
        return browser

    yield sign_in
    for browser in opened:
        browser.close()


@pytest.fixture
def owner(people) -> Browser:
    """The test's main user, signed in on the web."""
    return people("Functional Tester")


@pytest.fixture
def api(owner: Browser) -> Browser:
    """The owner's browser session, for API calls."""
    return owner


@pytest.fixture
def user(owner: Browser) -> str:
    """A ``pcl_`` token for the owner (what the CLI holds after `parcel login`)."""
    return owner.token("cli")


@pytest.fixture
def stranger(people) -> Browser:
    """A second signed-in user, who owns none of the test's parcels."""
    return people("Sam Stranger")


def content_token(content_url: str) -> str:
    """The capability token in a content URL (``…/v/<token>/<path>``)."""
    return content_url.split("/v/", 1)[1].split("/", 1)[0]


def query_of(url: str) -> dict[str, str]:
    return {k: v[0] for k, v in parse_qs(urlsplit(url).query).items()}


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
