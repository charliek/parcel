"""The HTTP API and the content origin, driven over real sockets."""

from __future__ import annotations

import json
from urllib.parse import urlsplit

import httpx

from conftest import Server

PAGE = (
    "<!doctype html><html><head><title>Functional page</title>"
    '<link rel="stylesheet" href="assets/site.css"></head>'
    '<body><h1>Hello</h1><script type="module" src="assets/app.mjs"></script></body></html>'
)


def publish(api: httpx.Client, files: dict[str, bytes | str], metadata: dict | None = None):
    parts = []
    if metadata is not None:
        parts.append(("metadata", (None, json.dumps(metadata), "application/json")))
    for path, data in files.items():
        parts.append(("file", (path, data, "application/octet-stream")))
    return api.post("/api/v1/parcels", files=parts)


def new_version(api: httpx.Client, parcel_id: str, files: dict, metadata: dict | None = None):
    parts = []
    if metadata is not None:
        parts.append(("metadata", (None, json.dumps(metadata), "application/json")))
    for path, data in files.items():
        parts.append(("file", (path, data, "application/octet-stream")))
    return api.post(f"/api/v1/parcels/{parcel_id}/versions", files=parts)


def test_info_and_me(server: Server, api: httpx.Client):
    info = httpx.get(f"{server.app_url}/api/v1/info").json()
    assert info["auth"]["mode"] == "slauth"
    assert info["content_origin"] == server.content_url
    me = api.get("/api/v1/me").json()
    assert me["usage"]["parcel_count"] == 0
    assert me["limits"]["max_file_bytes"] == 1024 * 1024
    assert httpx.get(f"{server.app_url}/api/v1/me").status_code == 401


def test_publish_view_and_serve_multi_file_parcel(server: Server, api: httpx.Client):
    response = publish(
        api,
        {
            "index.html": PAGE,
            "assets/site.css": "h1 { color: rebeccapurple }",
            "assets/app.mjs": "export const x = 1;",
            "img/logo.svg": '<svg xmlns="http://www.w3.org/2000/svg"/>',
        },
    )
    assert response.status_code == 201, response.text
    result = response.json()
    parcel = result["parcel"]
    assert parcel["title"] == "Functional page"
    assert parcel["url"] == f"{server.app_url}/p/{parcel['id']}"

    grant = api.get(f"/api/v1/parcels/{parcel['id']}/view").json()
    content_url = grant["content_url"]
    assert content_url.startswith(f"{server.content_url}/v/")

    with server.content() as content:
        page = content.get(content_url)
        assert page.status_code == 200
        assert page.text == PAGE
        csp = page.headers["content-security-policy"]
        assert csp.startswith("sandbox allow-scripts")
        assert "allow-same-origin" not in csp
        assert f"frame-ancestors {server.app_url}" in csp
        assert page.headers["referrer-policy"] == "no-referrer"
        assert "set-cookie" not in page.headers

        base = content_url.rsplit("/", 1)[0]
        module = content.get(f"{base}/assets/app.mjs", headers={"Origin": "null"})
        # Module scripts are CORS fetches from the sandbox's opaque origin.
        assert module.headers["access-control-allow-origin"] == "*"
        assert module.headers["content-type"].startswith("text/javascript")
        svg = content.get(f"{base}/img/logo.svg")
        assert svg.headers["content-type"] == "image/svg+xml"

        # The bytes really live in the GCS emulator.
        sha = next(f["sha256"] for f in result["version"]["files"] if f["path"] == "index.html")
        name = f"functional/blobs/sha256/{sha[:2]}/{sha}".replace("/", "%2F")
        stored = httpx.get(
            f"http://localhost:4447/storage/v1/b/{server.bucket}/o/{name}?alt=media"
        )
        assert stored.status_code == 200
        assert stored.text == PAGE


def test_origins_are_isolated(server: Server, api: httpx.Client):
    parcel = publish(api, {"index.html": PAGE}).json()["parcel"]
    grant = api.get(f"/api/v1/parcels/{parcel['id']}/view").json()
    path = urlsplit(grant["content_url"]).path
    # Content paths 404 on the app origin…
    assert httpx.get(f"{server.app_url}{path}").status_code == 404
    # …and the API 404s on the content origin.
    token = server.issuer.token()
    on_content = httpx.get(
        f"{server.content_url}/api/v1/me", headers={"Authorization": f"Bearer {token}"}
    )
    assert on_content.status_code == 404
    # Raw files on the app origin are inert attachments.
    raw = api.get(f"/api/v1/parcels/{parcel['id']}/versions/latest/files/index.html")
    assert raw.status_code == 200
    assert raw.headers["content-disposition"].startswith("attachment")
    assert raw.headers["content-security-policy"].startswith("sandbox")
    assert raw.text == PAGE


def test_versions_visibility_and_delete(server: Server, api: httpx.Client):
    parcel = publish(api, {"index.html": PAGE, "assets/site.css": "a{}"}).json()["parcel"]
    pid = parcel["id"]

    v2 = new_version(api, pid, {"assets/site.css": "b{}"}, {"label": "restyle"})
    assert v2.status_code == 201, v2.text
    assert v2.json()["version"]["version"] == 2
    assert v2.json()["parcel"]["url"] == parcel["url"]

    conflict = new_version(api, pid, {"x.css": "c{}"}, {"base_version": 1})
    assert conflict.status_code == 409

    versions = api.get(f"/api/v1/parcels/{pid}/versions").json()["versions"]
    assert [v["version"] for v in versions] == [2, 1]
    old_css = api.get(f"/api/v1/parcels/{pid}/versions/1/files/assets/site.css")
    assert old_css.text == "a{}"

    stranger = server.client(server.issuer.token())
    anonymous = server.client()
    assert stranger.get(f"/api/v1/parcels/{pid}").status_code == 404
    assert anonymous.get(f"/api/v1/parcels/{pid}/view").status_code == 404

    assert api.patch(f"/api/v1/parcels/{pid}", json={"visibility": "public"}).status_code == 200
    grant = anonymous.get(f"/api/v1/parcels/{pid}/view").json()
    assert grant["is_owner"] is False
    with server.content() as content:
        assert content.get(grant["content_url"]).status_code == 200
        assert stranger.patch(f"/api/v1/parcels/{pid}", json={"title": "x"}).status_code == 403

        api.patch(f"/api/v1/parcels/{pid}", json={"visibility": "private"})
        assert content.get(grant["content_url"]).status_code == 403

        owner_grant = api.get(f"/api/v1/parcels/{pid}/view?version=1").json()
        assert owner_grant["version"]["version"] == 1
        assert api.delete(f"/api/v1/parcels/{pid}").status_code == 204
        assert content.get(owner_grant["content_url"]).status_code == 404
    assert api.get(f"/api/v1/parcels/{pid}").status_code == 404


def test_limits_and_validation(api: httpx.Client):
    too_big = publish(api, {"index.html": "ok", "big.js": b"x" * (1024 * 1024 + 1)})
    assert too_big.status_code == 413
    assert "limit" in too_big.json()["error"]["message"]
    unsupported = publish(api, {"index.html": "ok", "run.sh": "echo"})
    assert unsupported.status_code == 400
    assert "unsupported file type" in unsupported.json()["error"]["message"]
    traversal = publish(api, {"../x.html": "ok"})
    assert traversal.status_code == 400


def test_tokens_and_device_login(server: Server, api: httpx.Client):
    created = api.post("/api/v1/tokens", json={"name": "functional"}).json()
    secret = created["secret"]
    with server.client(secret) as via_token:
        assert via_token.get("/api/v1/me").status_code == 200
        assert via_token.delete("/api/v1/tokens/current").status_code == 204
        assert via_token.get("/api/v1/me").status_code == 401

    anonymous = server.client()
    started = anonymous.post("/api/v1/cli/device", json={"client_name": "functional"}).json()
    poll = lambda: anonymous.post(  # noqa: E731
        "/api/v1/cli/device/token", json={"device_code": started["device_code"]}
    ).json()
    assert poll()["status"] == "pending"
    info = api.get(f"/api/v1/cli/device/{started['user_code']}").json()
    assert info["client_name"] == "functional"
    assert api.post(f"/api/v1/cli/device/{started['user_code']}/approve").status_code == 204
    approved = poll()
    assert approved["status"] == "approved"
    assert approved["token"].startswith("pcl_")
    assert poll()["status"] == "expired"


def test_markdown_entry(server: Server, api: httpx.Client):
    parcel = publish(api, {"README.md": "# Release notes\n\n* one\n* two\n"}).json()["parcel"]
    assert parcel["title"] == "Release notes"
    grant = api.get(f"/api/v1/parcels/{parcel['id']}/view").json()
    with server.content() as content:
        page = content.get(grant["content_url"])
    assert page.headers["content-type"] == "text/html; charset=utf-8"
    assert "<li>one</li>" in page.text


def test_openapi_and_docs(server: Server):
    spec = httpx.get(f"{server.app_url}/openapi.json").json()
    assert spec["info"]["title"] == "Parcel API"
    assert "/api/v1/parcels" in spec["paths"]
    docs = httpx.get(f"{server.app_url}/docs/", follow_redirects=True)
    assert docs.status_code == 200
    assert "swagger" in docs.text.lower()
