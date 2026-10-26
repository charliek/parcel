"""Tags, archive, full-text search and the unified list, over real sockets
(and the GCS emulator, which the post-publish indexing reads back from)."""

from __future__ import annotations

import json
import random
import string
import time

import httpx

from conftest import Browser, Cli, unique_email
from test_api import new_version, publish


def ids(listing: dict) -> list[str]:
    return [p["id"] for p in listing["parcels"]]


def found(api: httpx.Client, q: str, **params) -> dict:
    """Search, waiting briefly for the post-publish indexing to catch up."""
    deadline = time.monotonic() + 10
    while True:
        listing = api.get("/api/v1/parcels", params={"q": q, **params}).json()
        if listing["total"] or time.monotonic() > deadline:
            return listing
        time.sleep(0.1)


def word(prefix: str) -> str:
    """A word of letters only (full-text search splits on digits), unique to this run."""
    return prefix + "".join(random.choices(string.ascii_lowercase, k=10))


def test_server_lists_its_features(server):
    info = httpx.get(f"{server.app_url}/api/v1/info").json()
    assert {"tags", "search", "archive", "slugs", "pages", "search-relaxed"} <= set(info["features"])


def test_tags_filter_count_rename_and_delete(api: Browser, stranger: Browser):
    def tagged(title: str, tags: list[str]) -> str:
        r = publish(api, {"index.html": "<p>x</p>"}, {"title": title, "tags": tags})
        assert r.status_code == 201, r.text
        return r.json()["parcel"]["id"]

    ab = tagged("ab", ["A", "b", "project:x"])
    a = tagged("a", ["a"])
    tagged("c", ["c", "topic:x/y"])
    theirs = publish(stranger, {"index.html": "<p>x</p>"}, {"tags": ["a"]}).json()["parcel"]["id"]

    assert api.get(f"/api/v1/parcels/{ab}").json()["parcel"]["tags"] == ["a", "b", "project:x"]
    bad = publish(api, {"index.html": "<p>x</p>"}, {"tags": ["no good!"]})
    assert bad.status_code == 400 and '"no good!"' in bad.json()["error"]["message"]

    # Repeated `tag` parameters: all must match.
    both = api.get("/api/v1/parcels", params=[("tag", "a"), ("tag", "b")]).json()
    assert (ids(both), both["total"]) == ([ab], 1)
    counts = api.get("/api/v1/tags", params={"tag": "a"}).json()
    assert counts["tags"] == [
        {"tag": "a", "count": 2},
        {"tag": "b", "count": 1},
        {"tag": "project:x", "count": 1},
    ]
    assert api.get("/api/v1/tags", params={"prefix": "topic:"}).json()["tags"] == [
        {"tag": "topic:x/y", "count": 1}
    ]

    # PATCH edits; organizing doesn't touch updated_at or the order.
    before = api.get("/api/v1/parcels").json()
    edited = api.patch(f"/api/v1/parcels/{a}", json={"add_tags": ["d"], "remove_tags": ["a"]})
    assert edited.status_code == 200, edited.text
    assert edited.json()["tags"] == ["d"]
    assert api.patch(f"/api/v1/parcels/{a}", json={"tags": ["x"], "add_tags": ["y"]}).status_code == 400
    renamed = api.post("/api/v1/tags/rename", json={"from": "b", "to": "d"})
    assert renamed.json() == {"updated": 1}
    deleted = api.post("/api/v1/tags/delete", json={"tag": "topic:x/y"})
    assert deleted.json() == {"updated": 1}
    after = api.get("/api/v1/parcels").json()
    assert [(p["id"], p["updated_at"]) for p in after["parcels"]] == [
        (p["id"], p["updated_at"]) for p in before["parcels"]
    ]
    assert stranger.get(f"/api/v1/parcels/{theirs}").json()["parcel"]["tags"] == ["a"]


def test_archive_hides_until_asked_or_republished(api: Browser):
    pid = publish(api, {"index.html": "<p>x</p>"}, {"title": "Archivable"}).json()["parcel"]["id"]
    archived = api.patch(f"/api/v1/parcels/{pid}", json={"archived": True}).json()
    assert archived["archived_at"]
    assert pid not in ids(api.get("/api/v1/parcels").json())
    assert pid in ids(api.get("/api/v1/parcels", params={"archived": "include"}).json())
    assert ids(api.get("/api/v1/parcels", params={"archived": "only"}).json()) == [pid]
    assert api.get(f"/api/v1/parcels/{pid}").status_code == 200, "still viewable by URL"

    republished = new_version(api, pid, {"index.html": "<p>v2</p>"})
    assert republished.status_code == 201, republished.text
    assert republished.json()["parcel"]["archived_at"] is None
    assert pid in ids(api.get("/api/v1/parcels").json())


def test_full_text_search_finds_page_text(api: Browser):
    pid = publish(
        api,
        {"index.html": "<h1>Plain title</h1><p>The axolotl migration is on track.</p>"},
        {"title": "Status"},
    ).json()["parcel"]["id"]
    hit = found(api, "axolotl")
    assert ids(hit) == [pid]
    segments = hit["parcels"][0]["match"]["segments"]
    assert [s["text"] for s in segments if s["hit"]] == ["axolotl"]

    # A new version re-indexes from the new page.
    assert new_version(api, pid, {"index.html": "<p>Now about the pangolin.</p>"}, {"mode": "replace"}).status_code == 201
    assert ids(found(api, "pangolin")) == [pid]
    assert api.get("/api/v1/parcels", params={"q": "axolotl"}).json()["total"] == 0


def test_shared_and_all_scopes(api: Browser, people):
    bea_email = unique_email("Bea Scoped")
    shared = publish(api, {"index.html": "<p>x</p>"}, {"title": "For Bea", "tags": ["review"]})
    sid = shared.json()["parcel"]["id"]
    private = publish(api, {"index.html": "<p>x</p>"}, {"title": "Not for Bea", "tags": ["review"]})
    internal = publish(
        api, {"index.html": "<p>x</p>"}, {"title": "Internal", "visibility": "internal", "tags": ["review"]}
    )
    assert api.put(f"/api/v1/parcels/{sid}/shares/{bea_email}", json={"role": "commenter"}).status_code == 200
    bea = people("Bea Scoped", email=bea_email)
    own = publish(bea, {"index.html": "<p>x</p>"}, {"title": "Bea's own", "tags": ["review"]}).json()["parcel"]["id"]

    listing = bea.get("/api/v1/parcels", params={"scope": "shared"}).json()
    assert (ids(listing), listing["total"]) == ([sid], 1)
    assert listing["parcels"][0]["shared"]["role"] == "commenter"
    assert listing["parcels"][0]["shared"]["owner_name"] == "Functional Tester"
    everything = bea.get("/api/v1/parcels", params={"scope": "all", "tag": "review"}).json()
    assert sorted(ids(everything)) == sorted([sid, own])
    for p in (private, internal):
        assert p.json()["parcel"]["id"] not in ids(everything)
    assert bea.get("/api/v1/tags", params={"scope": "all"}).json()["tags"] == [{"tag": "review", "count": 2}]

    # Archived shared parcels leave the new list but stay in the legacy one.
    api.patch(f"/api/v1/parcels/{sid}", json={"archived": True})
    assert bea.get("/api/v1/parcels", params={"scope": "shared"}).json()["total"] == 0
    assert [s["parcel"]["id"] for s in bea.get("/api/v1/shared").json()["parcels"]] == [sid]


def test_the_current_cli_lists_tagged_and_archived_parcels(cli: Cli, api: Browser, user: str):
    pid = publish(api, {"index.html": "<p>x</p>"}, {"title": "Tagged for the CLI", "tags": ["kind:plan"]})
    pid = pid.json()["parcel"]["id"]
    gone = publish(api, {"index.html": "<p>x</p>"}, {"title": "Archived for the CLI"}).json()["parcel"]["id"]
    api.patch(f"/api/v1/parcels/{gone}", json={"archived": True})
    out = cli.run("list", token=user).stdout
    assert "Tagged for the CLI" in out and "Archived for the CLI" not in out
    listed = cli.json("list", token=user)
    assert [p["tags"] for p in listed["parcels"] if p["id"] == pid] == [["kind:plan"]]


def test_a_search_nothing_matches_exactly_relaxes(cli: Cli, api: Browser, user: str):
    # Titles are searchable as soon as the publish commits.
    a, b = word("ka"), word("kb")
    first = publish(api, {"index.html": "<p>x</p>"}, {"title": f"Notes on {a}", "tags": ["birds"]})
    second = publish(api, {"index.html": "<p>x</p>"}, {"title": f"Notes on {b}"})
    expected = sorted(r.json()["parcel"]["id"] for r in (first, second))

    listing = api.get("/api/v1/parcels", params={"q": f"{a} {b}"}).json()
    assert listing["relaxed"] is True and sorted(ids(listing)) == expected
    assert "relaxed" not in api.get("/api/v1/parcels", params={"q": a}).json()
    tags = api.get("/api/v1/tags", params={"q": f"{a} {b}"}).json()
    assert tags == {"tags": [{"tag": "birds", "count": 1}], "total": 1, "relaxed": True}

    # The CLI: the body as is on stdout with --json, the notice on stderr only.
    out = cli.run("list", "-q", f"{a} {b}", "--json", token=user)
    assert json.loads(out.stdout)["relaxed"] is True and out.stderr == ""
    out = cli.run("list", "-q", f"{a} {b}", token=user)
    assert out.stderr == "No exact matches; showing parcels that match any of the words.\n"
    assert a in out.stdout and b in out.stdout and "No exact matches" not in out.stdout
    out = cli.run("list", "-q", a, token=user)
    assert out.stderr == "" and a in out.stdout
