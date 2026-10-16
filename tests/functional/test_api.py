"""The HTTP API and the content origin, driven over real sockets."""

from __future__ import annotations

import json
import re
from urllib.parse import urlsplit

import httpx

from conftest import MAX_DATA_BYTES, MAX_DATA_VALUE_BYTES, Server, content_token

PAGE = (
    "<!doctype html><html><head><title>Functional page</title>"
    '<link rel="stylesheet" href="assets/site.css"></head>'
    '<body><h1>Hello</h1><script type="module" src="assets/app.mjs"></script></body></html>'
)

# The page helper the content origin adds to every HTML page it serves.
HELPER_TAG = re.compile(r'<script src="(/_parcel/helper\.([0-9a-f]{12})\.js)"></script>')


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
    assert info["auth"]["mode"] == "oidc"
    assert info["content_origin"] == server.content_url
    me = api.get("/api/v1/me").json()
    assert me["usage"]["parcel_count"] == 0
    assert me["limits"]["max_file_bytes"] == 1024 * 1024
    assert info["limits"]["max_data_value_bytes"] == MAX_DATA_VALUE_BYTES
    assert info["limits"]["max_data_bytes"] == MAX_DATA_BYTES
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
        # The stored page, plus the helper script right after <head>.
        tag = HELPER_TAG.search(page.text)
        assert tag, page.text
        assert page.text == PAGE.replace("<head>", f"<head>{tag.group(0)}", 1)
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


def test_origins_are_isolated(server: Server, api: httpx.Client, user: str):
    parcel = publish(api, {"index.html": PAGE}).json()["parcel"]
    grant = api.get(f"/api/v1/parcels/{parcel['id']}/view").json()
    path = urlsplit(grant["content_url"]).path
    # Content paths 404 on the app origin…
    assert httpx.get(f"{server.app_url}{path}").status_code == 404
    # …and the API and sign-in 404 on the content origin.
    on_content = httpx.get(
        f"{server.content_url}/api/v1/me", headers={"Authorization": f"Bearer {user}"}
    )
    assert on_content.status_code == 404
    assert httpx.get(f"{server.content_url}/auth/login").status_code == 404
    # Raw files on the app origin are inert attachments.
    raw = api.get(f"/api/v1/parcels/{parcel['id']}/versions/latest/files/index.html")
    assert raw.status_code == 200
    assert raw.headers["content-disposition"].startswith("attachment")
    assert raw.headers["content-security-policy"].startswith("sandbox")
    assert raw.text == PAGE


def test_versions_visibility_and_delete(
    server: Server, api: httpx.Client, stranger: httpx.Client
):
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


# ---------------------------------------------------------------- comments


ANCHOR = {
    "path": "index.html",
    "selector": "html > body > h1",
    "text": "Hello",
    "x": 0.5,
    "y": 0.25,
}


def test_comment_threads_and_permissions(
    server: Server, api: httpx.Client, stranger: httpx.Client
):
    pid = publish(api, {"index.html": PAGE}).json()["parcel"]["id"]
    base = f"/api/v1/parcels/{pid}/comments"
    anonymous = server.client()

    created = api.post(base, json={"body": "  Make it bigger ", "anchor": ANCHOR})
    assert created.status_code == 201, created.text
    first = created.json()
    assert (first["number"], first["version"], first["body"]) == (1, 1, "Make it bigger")
    assert first["anchor"] == ANCHOR
    assert first["via"] == "web"
    assert first["author"] == {"name": "Functional Tester", "is_owner": True, "is_you": True}

    # A private parcel's comments are 404 to everyone else, never 403.
    assert stranger.get(base).status_code == 404
    assert stranger.post(base, json={"body": "hi"}).status_code == 404
    assert anonymous.get(base).status_code == 404
    assert anonymous.post(base, json={"body": "hi"}).status_code == 401

    # Public: any signed-in user joins in; signed-out viewers see none.
    assert api.patch(f"/api/v1/parcels/{pid}", json={"visibility": "public"}).status_code == 200
    assert anonymous.get(base).status_code == 401
    seen = stranger.get(base).json()
    assert seen["open"] == 1
    assert seen["comments"][0]["author"] == {
        "name": "Functional Tester",
        "is_owner": True,
        "is_you": False,
    }
    theirs = stranger.post(base, json={"body": "Typo in the heading", "anchor": ANCHOR}).json()
    assert theirs["number"] == 2
    assert theirs["author"] == {"name": "Sam Stranger", "is_owner": False, "is_you": True}
    answered = api.post(base, json={"body": "Which word?", "reply_to": 2})
    assert answered.status_code == 201
    assert [(r["number"], r["author"]["is_owner"]) for r in answered.json()["replies"]] == [
        (3, True)
    ]

    # Shape rules.
    assert api.post(base, json={"body": "x", "reply_to": 3}).status_code == 400  # a reply's reply
    assert api.post(base, json={"body": "x", "reply_to": 2, "anchor": ANCHOR}).status_code == 400
    assert api.post(base, json={"body": "   "}).status_code == 400
    assert api.post(base, json={"body": "x", "version": 9}).status_code == 400
    assert api.post(base, json={"body": "x", "reply_to": 99}).status_code == 404
    assert api.patch(f"{base}/3", json={"resolved": True}).status_code == 400  # replies aren't
    assert api.patch(f"{base}/99", json={"resolved": True}).status_code == 404

    # Only the author edits; the owner or the thread's author resolves; the
    # author or the owner deletes.
    assert stranger.patch(f"{base}/1", json={"body": "edited"}).status_code == 403
    assert stranger.patch(f"{base}/1", json={"resolved": True}).status_code == 403
    assert stranger.delete(f"{base}/1").status_code == 403
    assert api.patch(f"{base}/2", json={"body": "owner rewrite"}).status_code == 403
    edited = stranger.patch(f"{base}/2", json={"body": "Typo: 'Helo'"}).json()
    assert edited["body"] == "Typo: 'Helo'"
    resolved = stranger.patch(f"{base}/2", json={"resolved": True}).json()
    assert resolved["resolved"] is True and resolved["resolved_at"]
    assert api.patch(f"{base}/2", json={"resolved": False}).json()["resolved"] is False
    assert api.patch(f"{base}/1", json={"resolved": True}).status_code == 200

    # Filters; `open` counts unresolved threads whatever the filter.
    def numbers(status: str) -> list[int]:
        return [c["number"] for c in api.get(base, params={"status": status}).json()["comments"]]

    assert [c["number"] for c in api.get(base).json()["comments"]] == [2]
    assert numbers("resolved") == [1]
    assert numbers("all") == [1, 2]
    assert api.get(base, params={"status": "resolved"}).json()["open"] == 1
    assert api.get(f"/api/v1/parcels/{pid}").json()["parcel"]["open_comments"] == 1
    assert [p["open_comments"] for p in api.get("/api/v1/parcels").json()["parcels"]] == [1]

    # A pcl_ token (the CLI, agents) comments `via: cli`; numbers never repeat.
    secret = api.post("/api/v1/tokens", json={"name": "agent"}).json()["secret"]
    with server.client(secret) as agent:
        via_cli = agent.post(base, json={"body": "Fixed in v2", "reply_to": 2}).json()
    assert (via_cli["replies"][-1]["number"], via_cli["replies"][-1]["via"]) == (4, "cli")

    # The owner deletes anyone's comment; a thread takes its replies along.
    assert api.delete(f"{base}/2").status_code == 204
    assert numbers("all") == [1]
    assert api.delete(f"{base}/2").status_code == 404
    assert api.post(base, json={"body": "next"}).json()["number"] == 5
    anonymous.close()


def test_activity_tracks_versions_and_comments(server: Server, api: httpx.Client):
    pid = publish(api, {"index.html": PAGE}).json()["parcel"]["id"]
    url = f"/api/v1/parcels/{pid}/activity"
    response = api.get(url)
    assert response.status_code == 200
    assert response.headers["cache-control"] == "no-store"
    start = response.json()
    assert (start["current_version"], start["open_comments"]) == (1, 0)

    api.post(f"/api/v1/parcels/{pid}/comments", json={"body": "Looks off"})
    commented = api.get(url).json()
    assert commented["open_comments"] == 1
    assert commented["comments_revision"] != start["comments_revision"]

    assert new_version(api, pid, {"index.html": PAGE + "<p>v2</p>"}).status_code == 201
    published = api.get(url).json()
    assert published["current_version"] == 2
    assert published["updated_at"] != start["updated_at"]

    api.patch(f"/api/v1/parcels/{pid}/comments/1", json={"resolved": True})
    resolved = api.get(url).json()
    assert resolved["open_comments"] == 0
    assert resolved["comments_revision"] != published["comments_revision"]

    with server.client() as anonymous:
        assert anonymous.get(url).status_code == 404
        api.patch(f"/api/v1/parcels/{pid}", json={"visibility": "public"})
        assert anonymous.get(url).json()["current_version"] == 2


# ---------------------------------------------------------------- data


def test_data_through_the_api(server: Server, api: httpx.Client, stranger: httpx.Client):
    pid = publish(api, {"index.html": PAGE}).json()["parcel"]["id"]
    base = f"/api/v1/parcels/{pid}/data"
    empty = api.get(base).json()
    assert (empty["collections"], empty["usage_bytes"]) == ([], 0)
    assert (empty["limit_bytes"], empty["writable"]) == (MAX_DATA_BYTES, True)

    put = api.put(f"{base}/votes/alice", json={"choice": "red", "n": 1})
    assert put.status_code == 200, put.text
    item = put.json()
    assert (item["collection"], item["key"]) == ("votes", "alice")
    assert item["value"] == {"choice": "red", "n": 1}
    assert item["size_bytes"] == len(b'{"n":1,"choice":"red"}')
    # Any JSON value, not only objects.
    assert api.put(f"{base}/votes/bob", content=b'"blue"').json()["value"] == "blue"
    assert api.put(f"{base}/votes/carol", json=3).json()["value"] == 3
    assert api.get(f"{base}/votes/bob").json()["value"] == "blue"

    # PATCH is an RFC 7386 merge: null removes; missing items are created.
    merged = api.patch(f"{base}/votes/alice", json={"n": 2, "choice": None, "tags": ["x"]})
    assert merged.json()["value"] == {"n": 2, "tags": ["x"]}
    assert api.patch(f"{base}/votes/dave", json={"a": 1}).json()["value"] == {"a": 1}
    assert api.patch(f"{base}/votes/dave", json=[1]).status_code == 400

    # Pages of a collection, by key.
    page1 = api.get(f"{base}/votes", params={"limit": 2}).json()
    assert [i["key"] for i in page1["items"]] == ["alice", "bob"]
    assert page1["next_after"] == "bob"
    page2 = api.get(f"{base}/votes", params={"limit": 2, "after": "bob"}).json()
    assert [i["key"] for i in page2["items"]] == ["carol", "dave"]
    assert "next_after" not in page2

    summary = api.get(base).json()
    assert [(c["name"], c["count"]) for c in summary["collections"]] == [("votes", 4)]
    sizes = [i["size_bytes"] for i in page1["items"] + page2["items"]]
    assert summary["usage_bytes"] == sum(sizes) + 4 * 128

    # Validation and limits.
    assert api.get(f"{base}/votes/zed").status_code == 404
    assert api.get(f"{base}/votes/zed").json()["error"]["message"] == "no item votes/zed"
    assert api.put(f"{base}/-votes/x", json=1).status_code == 400
    assert api.put(f"{base}/votes/a b", json=1).status_code == 400
    assert api.put(f"{base}/votes/x", content=b"{not json").status_code == 400
    too_big = api.put(f"{base}/big/one", json="x" * MAX_DATA_VALUE_BYTES)
    assert too_big.status_code == 413
    assert "at most" in too_big.json()["error"]["message"]
    # Fill the parcel: writes succeed until its data limit, then 413.
    chunk = "y" * (MAX_DATA_VALUE_BYTES - 512)
    statuses = [api.put(f"{base}/big/{n}", json=chunk).status_code for n in range(6)]
    assert statuses[0] == 200 and statuses[-1] == 413 and statuses == sorted(statuses), statuses
    assert "limit" in api.put(f"{base}/big/9", json=chunk).json()["error"]["message"]
    assert api.get(base).json()["usage_bytes"] <= MAX_DATA_BYTES
    assert api.delete(f"{base}/big").status_code == 204

    # Private: 404 to others. Public: anyone reads, only the owner writes.
    assert stranger.get(base).status_code == 404
    assert stranger.put(f"{base}/votes/eve", json=1).status_code == 404
    api.patch(f"/api/v1/parcels/{pid}", json={"visibility": "public"})
    with server.client() as anonymous:
        visible = anonymous.get(base).json()
        assert visible["writable"] is False
        assert anonymous.get(f"{base}/votes/carol").json()["value"] == 3
        assert anonymous.put(f"{base}/votes/eve", json=1).status_code == 401
    assert stranger.get(base).json()["writable"] is False
    assert stranger.put(f"{base}/votes/eve", json=1).status_code == 403
    assert stranger.delete(f"{base}/votes").status_code == 403

    assert api.delete(f"{base}/votes/carol").status_code == 204
    assert api.get(f"{base}/votes/carol").status_code == 404
    assert api.delete(f"{base}/votes/carol").status_code == 204  # already gone is fine
    assert api.delete(f"{base}/votes").status_code == 204
    assert api.get(base).json()["collections"] == []


def test_content_origin_helper_and_page_data(server: Server, api: httpx.Client):
    pid = publish(api, {"index.html": PAGE, "bare.html": "<p>no head</p>"}).json()["parcel"]["id"]
    owner_url = api.get(f"/api/v1/parcels/{pid}/view").json()["content_url"]
    owner_data = f"{server.content_url}/d/{content_token(owner_url)}"

    with server.content() as content:
        page = content.get(owner_url)
        tag = HELPER_TAG.search(page.text)
        assert tag and page.text.index(tag.group(0)) == PAGE.index("<head>") + len("<head>")
        assert tag.group(2) in page.headers["etag"]
        bare = content.get(owner_url.rsplit("/", 1)[0] + "/bare.html")
        assert bare.text == tag.group(0) + "<p>no head</p>"
        # The raw-file API on the app origin still returns the stored bytes.
        raw = api.get(f"/api/v1/parcels/{pid}/versions/latest/files/index.html")
        assert raw.text == PAGE

        helper = content.get(f"{server.content_url}{tag.group(1)}")
        assert helper.status_code == 200
        assert helper.headers["content-type"].startswith("text/javascript")
        assert "immutable" in helper.headers["cache-control"]
        assert helper.headers["access-control-allow-origin"] == "*"
        assert f'const APP_ORIGIN = "{server.app_url}";' in helper.text
        assert "defineProperty(window, 'parcel'" in helper.text
        stale = content.get(f"{server.content_url}/_parcel/helper.000000000000.js")
        assert stale.headers["cache-control"] == "no-cache"
        assert httpx.get(f"{server.app_url}{tag.group(1)}").status_code == 404

        # The owner's page reads and writes its data, cross-origin from `null`.
        cors = {"Origin": "null"}
        put = content.put(f"{owner_data}/todo/done", json={"0": True}, headers=cors)
        assert put.status_code == 200, put.text
        assert put.headers["access-control-allow-origin"] == "*"
        assert put.headers["cache-control"] == "no-store"
        merged = content.patch(f"{owner_data}/todo/done", json={"1": True}, headers=cors)
        assert merged.json()["value"] == {"0": True, "1": True}
        assert content.get(f"{owner_data}/todo/done").json()["value"] == {"0": True, "1": True}
        assert [i["key"] for i in content.get(f"{owner_data}/todo").json()["items"]] == ["done"]
        summary = content.get(owner_data).json()
        assert summary["writable"] is True
        assert [c["name"] for c in summary["collections"]] == ["todo"]
        # The same data through the API.
        assert api.get(f"/api/v1/parcels/{pid}/data/todo/done").json()["value"] == {
            "0": True,
            "1": True,
        }
        preflight = content.request(
            "OPTIONS",
            f"{owner_data}/todo/done",
            headers={
                **cors,
                "Access-Control-Request-Method": "PUT",
                "Access-Control-Request-Headers": "content-type",
            },
        )
        assert preflight.status_code == 204
        assert "PUT" in preflight.headers["access-control-allow-methods"]
        assert "content-type" in preflight.headers["access-control-allow-headers"]
        assert preflight.headers["access-control-allow-origin"] == "*"

        # A visitor's page (public grant) reads but can't write.
        api.patch(f"/api/v1/parcels/{pid}", json={"visibility": "public"})
        visitor_grant = httpx.get(f"{server.app_url}/api/v1/parcels/{pid}/view").json()
        assert visitor_grant["is_owner"] is False
        visitor_data = f"{server.content_url}/d/{content_token(visitor_grant['content_url'])}"
        assert content.get(f"{visitor_data}/todo/done").json()["value"] == {"0": True, "1": True}
        assert content.get(visitor_data).json()["writable"] is False
        denied = content.put(f"{visitor_data}/todo/done", json={"0": False}, headers=cors)
        assert denied.status_code == 403
        assert "only the parcel's owner" in denied.json()["error"]["message"]
        assert denied.headers["access-control-allow-origin"] == "*"  # the page can read why
        assert content.delete(f"{visitor_data}/todo").status_code == 403

        # Going private cuts the visitor off; forged tokens are not found.
        api.patch(f"/api/v1/parcels/{pid}", json={"visibility": "private"})
        assert content.get(f"{visitor_data}/todo/done").status_code == 403
        assert content.get(f"{server.content_url}/d/forged/todo").status_code == 404
        assert content.delete(f"{owner_data}/todo/done").status_code == 204
        assert content.get(f"{owner_data}/todo/done").status_code == 404
        # Page data lives only on the content origin.
        assert httpx.get(f"{server.app_url}/d/{content_token(owner_url)}").status_code == 404
