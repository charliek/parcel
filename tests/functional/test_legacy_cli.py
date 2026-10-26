"""The released CLI (``LEGACY_CLI_TAG``, built from the tag) against this
server, with slugged, tagged and archived parcels present: every response
it reads must still parse, and every URL it builds must still work."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from conftest import LEGACY_CLI_TAG, Browser, Cli, Server
from test_api import publish


def page(title: str) -> str:
    return f"<!doctype html><title>{title}</title><h1>{title}</h1>"


# The first run builds the released CLI (cached afterwards): allow for a cold build.
@pytest.mark.timeout(1200)
def test_the_released_cli_still_works(
    legacy_cli: Cli, server: Server, owner: Browser, user: str, stranger: Browser, tmp_path: Path
):
    version = legacy_cli.run("--version").stdout
    assert LEGACY_CLI_TAG.removeprefix("v") in version, version

    # What a current client leaves on the server: a slug, tags, a source,
    # an archived parcel, and a slugged, tagged parcel someone shared.
    slugged = publish(
        owner,
        {"index.html": page("Slugged plan")},
        {
            "slug": "legacy/plan",
            "tags": ["kind:plan", "project:legacy"],
            "source": {"repo": "https://github.com/o/r.git", "client": "parcel-cli/9.9.9"},
        },
    )
    assert slugged.status_code == 201, slugged.text
    slugged = slugged.json()["parcel"]
    pid = slugged["id"]
    assert slugged["canonical_url"].endswith("/legacy/plan")
    archived = publish(owner, {"index.html": page("Put away")}, {"tags": ["old"]}).json()["parcel"]
    assert owner.patch(f"/api/v1/parcels/{archived['id']}", json={"archived": True}).status_code == 200
    theirs = publish(
        stranger, {"index.html": page("Shared with me")}, {"slug": "theirs", "tags": ["x"]}
    ).json()["parcel"]
    shared = stranger.put(
        f"/api/v1/parcels/{theirs['id']}/shares/{owner.email}", json={"role": "viewer"}
    )
    assert shared.status_code == 200, shared.text

    # list: the table and the JSON both read the flattened items.
    table = legacy_cli.run("list", token=user).stdout
    assert "Slugged plan" in table and "Put away" not in table, table
    listed = legacy_cli.json("list", token=user)
    assert [p["id"] for p in listed["parcels"]] == [pid]
    assert listed["parcels"][0]["url"] == f"{server.app_url}/p/{pid}"
    # -q is full text now (the slug and tags match too); the table still reads it.
    assert "Slugged plan" in legacy_cli.run("list", "-q", "legacy", token=user).stdout
    # A relaxed list (no parcel has both words) parses too: the table shows
    # it without the notice, and --json passes `relaxed` through.
    assert owner.get("/api/v1/parcels", params={"q": "slugged nowhere"}).json()["relaxed"] is True
    relaxed = legacy_cli.run("list", "-q", "slugged nowhere", token=user)
    assert "Slugged plan" in relaxed.stdout and "No exact matches" not in relaxed.stderr, relaxed
    relaxed = legacy_cli.json("list", "-q", "slugged nowhere", token=user)
    assert relaxed["relaxed"] is True and [p["id"] for p in relaxed["parcels"]] == [pid]
    # list --shared: the legacy endpoint, unchanged.
    shared_table = legacy_cli.run("list", "--shared", token=user).stdout
    assert "Shared with me" in shared_table, shared_table
    assert [p["parcel"]["id"] for p in legacy_cli.json("list", "--shared", token=user)["parcels"]] == [
        theirs["id"]
    ]

    # info, by id and by URL, with and without a version.
    info = legacy_cli.run("info", pid, token=user).stdout
    assert "Slugged plan" in info and f"/p/{pid}" in info, info
    assert legacy_cli.json("info", pid, token=user)["parcel"]["id"] == pid
    pinned = legacy_cli.run("info", f"{server.app_url}/p/{pid}/v/1", token=user).stdout
    assert "index.html" in pinned, pinned
    assert "index.html" in legacy_cli.run("info", pid, "--version", "1", token=user).stdout

    # open prints the /p/<id> URL it builds (and appends /v/<n> to).
    assert legacy_cli.run("open", pid, token=user).stdout.strip() == f"{server.app_url}/p/{pid}"
    opened = legacy_cli.run("open", f"{server.app_url}/p/{pid}/v/1", token=user).stdout
    assert opened.strip() == f"{server.app_url}/p/{pid}/v/1"

    # cat reads the source.
    assert legacy_cli.run("cat", pid, token=user).stdout == page("Slugged plan")

    # publish --parcel versions the slugged parcel and the archived one.
    site = tmp_path / "site"
    site.mkdir()
    (site / "index.html").write_text(page("Second version"))
    out = legacy_cli.json("publish", str(site), "--parcel", pid, token=user)
    assert out["parcel"]["id"] == pid and out["version"]["version"] == 2
    after = owner.get(f"/api/v1/parcels/{pid}").json()
    assert after["parcel"]["slug"] == "legacy/plan"
    assert after["parcel"]["tags"] == ["kind:plan", "project:legacy"]
    assert after["versions"][0]["version"] == 2
    assert "source" not in after["versions"][0], "the released CLI sends none"
    assert after["versions"][1]["source"]["repo"] == "github.com/o/r"
    out = legacy_cli.json("publish", str(site), "--parcel", archived["id"], token=user)
    assert out["version"]["version"] == 2
    revived = owner.get(f"/api/v1/parcels/{archived['id']}").json()["parcel"]
    assert revived["archived_at"] is None, "a new version un-archives"
    assert legacy_cli.run("cat", f"{server.app_url}/p/{pid}/v/1", token=user).stdout == page(
        "Slugged plan"
    )
    # Its JSON output is the server's body: the new fields ride along.
    assert json.dumps(out).count("canonical_url") == 1
