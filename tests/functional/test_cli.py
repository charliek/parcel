"""The `parcel` CLI against the real server: login, publish, read, manage."""

from __future__ import annotations

import json
import re
import subprocess
import time
from pathlib import Path

import httpx

from conftest import CLI_BIN, MAX_DATA_VALUE_BYTES, ROOT, Cli, Server, content_token

PAGE = """<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>CLI report</title>
<link rel="stylesheet" href="assets/site.css"></head>
<body><h1>Numbers</h1><script src="assets/app.js"></script></body></html>
"""


def device_login(cli: Cli, server: Server, approver_token: str) -> None:
    """Run `parcel login` for real and approve it the way the web app does."""
    process = subprocess.Popen(
        [str(CLI_BIN), "login", "--no-browser"],
        env=cli.env(None),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    code = None
    assert process.stderr is not None
    deadline = time.time() + 20
    seen = ""
    while code is None and time.time() < deadline:
        line = process.stderr.readline()
        seen += line
        if match := re.search(r"confirm the code ([A-Z]{4}-[A-Z]{4})", line):
            code = match.group(1)
    assert code, f"no device code printed:\n{seen}"
    url = f"{server.app_url}/device?code={code}"
    while url not in seen and time.time() < deadline:
        seen += process.stderr.readline()
    assert url in seen, seen
    with server.client(approver_token) as api:
        assert api.get(f"/api/v1/cli/device/{code}").json()["user_code"] == code
        assert api.post(f"/api/v1/cli/device/{code}/approve").status_code == 204
    stdout, stderr = process.communicate(timeout=30)
    assert process.returncode == 0, stderr
    assert "Logged in" in stdout + stderr


def test_device_login_whoami_and_logout(cli: Cli, server: Server):
    approver = server.issuer.token(email="cli-user@example.com")
    assert cli.run("whoami", check=False).returncode == 3

    device_login(cli, server, approver)
    me = cli.json("whoami")
    assert me["email"] == "cli-user@example.com"

    config = cli.home / ".config" / "parcel" / "config.toml"
    assert config.stat().st_mode & 0o077 == 0, "config holds a token: must be 0600"
    assert "pcl_" in config.read_text()
    shown = cli.run("config", "show").stdout
    assert "pcl_" not in shown, "never print the token"

    cli.run("logout")
    assert "pcl_" not in config.read_text()
    assert cli.run("whoami", check=False).returncode == 3


def test_publish_read_update_and_delete(cli: Cli, server: Server, user: str, tmp_path: Path):
    site = tmp_path / "site"
    (site / "assets").mkdir(parents=True)
    (site / "index.html").write_text(PAGE)
    (site / "assets" / "site.css").write_text("h1 { color: teal }")
    (site / "assets" / "app.js").write_text("document.title += ' ✓'")
    (site / ".env").write_text("SECRET=1")  # dotfiles are never published
    (site / "node_modules").mkdir()
    (site / "node_modules" / "x.js").write_text("nope")

    created = cli.json("publish", str(site), "--label", "first", token=user)
    parcel = created["parcel"]
    pid = parcel["id"]
    assert parcel["title"] == "CLI report"
    assert parcel["url"] == f"{server.app_url}/p/{pid}"
    published = sorted(f["path"] for f in created["version"]["files"])
    assert published == ["assets/app.js", "assets/site.css", "index.html"]

    listed = cli.json("list", token=user)
    assert [p["id"] for p in listed["parcels"]] == [pid]
    human = cli.run("list", token=user).stdout
    assert pid in human and "CLI report" in human

    # Read it back, by id and by URL.
    assert cli.run("cat", pid, token=user).stdout == PAGE
    assert cli.run("cat", parcel["url"], "assets/site.css", token=user).stdout == "h1 { color: teal }"
    out = tmp_path / "pulled"
    cli.run("pull", pid, str(out), token=user)
    assert (out / "assets" / "app.js").read_text() == "document.title += ' ✓'"
    assert not (out / ".env").exists()

    # Merge a single file into a new version: the URL stays the same.
    patch = tmp_path / "data.json"
    patch.write_text('{"rows": 3}')
    v2 = cli.json(
        "publish", "--parcel", pid, "--file", f"data/rows.json={patch}", "--label", "data",
        token=user,
    )
    assert v2["version"]["version"] == 2
    assert v2["parcel"]["url"] == parcel["url"]
    assert sorted(f["path"] for f in v2["version"]["files"]) == [
        "assets/app.js", "assets/site.css", "data/rows.json", "index.html",
    ]
    assert cli.run("publish", "--parcel", pid, "--file", str(patch), "--base-version", "1",
                   token=user, check=False).returncode != 0

    versions = cli.json("versions", pid, token=user)
    assert [v["version"] for v in versions["versions"]] == [2, 1]
    assert cli.run("cat", f"{parcel['url']}/v/1", "assets/site.css", token=user).returncode == 0
    assert cli.run("cat", f"{parcel['url']}/v/1", "data/rows.json", token=user,
                   check=False).returncode == 4

    restored = cli.json("restore", pid, "--version", "1", token=user)
    assert restored["version"]["version"] == 3
    assert len(restored["version"]["files"]) == 3

    # Visibility: public parcels are readable by anyone.
    cli.json("update", pid, "--public", "--title", "Shared CLI report", token=user)
    anonymous = Cli(server=server, home=cli.home)
    assert anonymous.run("cat", pid).stdout == PAGE
    detail = anonymous.json("info", pid)
    assert detail["parcel"]["title"] == "Shared CLI report"
    assert detail["is_owner"] is False
    cli.json("update", pid, "--private", token=user)
    assert anonymous.run("cat", pid, check=False).returncode == 4

    # Delete needs --yes without a terminal.
    assert cli.run("delete", pid, token=user, check=False).returncode != 0
    cli.run("delete", pid, "--yes", token=user)
    assert cli.run("info", pid, token=user, check=False).returncode == 4


def test_single_file_and_markdown_publishes(cli: Cli, server: Server, user: str, tmp_path: Path):
    note = tmp_path / "notes.md"
    note.write_text("# Standup\n\n- shipped parcel\n")
    created = cli.json("publish", str(note), "--public", token=user)
    assert created["parcel"]["entry"] == "notes.md"
    assert created["parcel"]["title"] == "Standup"
    assert created["parcel"]["visibility"] == "public"
    grant = httpx.get(f"{server.app_url}/api/v1/parcels/{created['parcel']['id']}/view").json()
    page = httpx.get(grant["content_url"])
    assert "<h1>Standup</h1>" in page.text


def test_client_side_validation(cli: Cli, user: str, tmp_path: Path):
    big = tmp_path / "big"
    big.mkdir()
    (big / "index.html").write_text("<p>ok</p>")
    (big / "huge.js").write_bytes(b"x" * (1024 * 1024 + 1))  # functional limit is 1 MiB
    result = cli.run("publish", str(big), token=user, check=False)
    assert result.returncode == 1
    assert "huge.js" in result.stderr

    odd = tmp_path / "odd"
    odd.mkdir()
    (odd / "index.html").write_text("<p>ok</p>")
    (odd / "tool.exe").write_bytes(b"MZ")
    result = cli.run("publish", str(odd), token=user, check=False)
    assert result.returncode == 1
    assert "tool.exe" in result.stderr


def test_skill_install_matches_the_repo(cli: Cli):
    result = cli.run("skill", "install")
    installed = cli.home / ".claude" / "skills" / "parcel" / "SKILL.md"
    assert installed.exists(), result.stdout + result.stderr
    assert installed.read_text() == (ROOT / "skills" / "parcel" / "SKILL.md").read_text()
    refs = sorted(p.name for p in (installed.parent / "references").iterdir())
    assert refs == sorted(p.name for p in (ROOT / "skills" / "parcel" / "references").iterdir())

    cli.run("skill", "install", "--agents")
    assert (cli.home / ".agents" / "skills" / "parcel" / "SKILL.md").exists()
    assert cli.run("skill", "show").stdout.startswith("---\nname: parcel")


def test_json_output_is_the_api_shape(cli: Cli, server: Server, user: str, tmp_path: Path):
    page = tmp_path / "x.html"
    page.write_text("<title>Shape</title>")
    created = cli.json("publish", str(page), token=user)
    with server.client(user) as api:
        via_api = api.get(f"/api/v1/parcels/{created['parcel']['id']}").json()
    via_cli = cli.json("info", created["parcel"]["id"], token=user)
    assert json.dumps(via_cli, sort_keys=True) == json.dumps(via_api, sort_keys=True)


def test_comments_feedback_loop(cli: Cli, server: Server, user: str, tmp_path: Path):
    """What the skill teaches: list the feedback, fix, republish, resolve with a note."""
    # The owner's CLI holds a pcl_ token, as after `parcel login`.
    with server.client(user) as owner_api:
        secret = owner_api.post("/api/v1/tokens", json={"name": "cli"}).json()["secret"]
    page = tmp_path / "index.html"
    page.write_text(PAGE)
    created = cli.json("publish", str(page), "--public", token=secret)
    pid, url = created["parcel"]["id"], created["parcel"]["url"]

    # A reviewer comments in the web viewer, pinned to the heading.
    anchor = {"path": "index.html", "selector": "html > body > h1", "text": "Numbers"}
    with server.client(server.issuer.token(name="Rita Reviewer")) as reviewer:
        made = reviewer.post(
            f"/api/v1/parcels/{pid}/comments",
            json={"body": "Say which quarter these are for", "anchor": anchor},
        )
        assert made.status_code == 201, made.text

    assert "comments:    1 open" in cli.run("info", pid, token=secret).stdout
    assert "1 open" in cli.run("list", token=secret).stdout

    lines = cli.run("comments", "list", url, token=secret).stdout.splitlines()
    assert lines[0] == f"1 open thread on {pid}"
    assert lines[2].startswith('#1 · v1 · index.html › "Numbers" · Rita Reviewer · '), lines
    assert lines[3] == "    Say which quarter these are for"

    added = cli.run("comments", "add", pid, "Will fix today", token=secret).stdout
    assert added == f"Added comment #2 on {pid} (v1)\n"
    replied = cli.run(
        "comments", "reply", pid, "1", "-", token=secret, input="On it.\nv2 shortly.\n"
    ).stdout
    assert replied == "Replied to #1 as #3\n"
    listed = cli.json("comments", "list", pid, token=secret)
    assert listed["open"] == 2
    reply = listed["comments"][0]["replies"][0]
    assert reply["body"] == "On it.\nv2 shortly."
    assert (reply["via"], reply["author"]["is_you"]) == ("cli", True)

    # Fix it, publish to the same link, resolve with a note.
    page.write_text(PAGE.replace("<h1>Numbers</h1>", "<h1>Q3 numbers</h1>"))
    v2 = cli.json("publish", str(page), "--parcel", url, "--label", "Address comments", token=secret)
    assert (v2["version"]["version"], v2["parcel"]["url"]) == (2, url)
    note = "Fixed in v2: the heading says Q3"
    resolved = cli.run("comments", "resolve", pid, "1", "2", "--reply", note, token=secret)
    assert resolved.stdout == "Resolved #1 (replied as #4)\nResolved #2 (replied as #5)\n"
    assert cli.run("comments", "list", pid, token=secret).stdout == f"No open comments on {pid}.\n"
    every = cli.json("comments", "list", pid, "--status", "all", token=secret)
    assert [(c["number"], c["resolved"]) for c in every["comments"]] == [(1, True), (2, True)]
    assert every["comments"][0]["replies"][-1]["body"] == note
    assert "resolved" in cli.run("comments", "list", pid, "--status", "resolved", token=secret).stdout

    assert cli.run("comments", "reopen", pid, "2", token=secret).stdout == "Reopened #2\n"
    assert cli.json("comments", "list", pid, token=secret)["open"] == 1
    deleted = cli.run("comments", "delete", pid, "2", token=secret).stdout
    assert deleted == f"Deleted comment #2 from {pid}\n"
    assert cli.json("comments", "list", pid, token=secret)["open"] == 0

    # Errors and exit codes.
    assert cli.run("comments", "resolve", pid, "42", token=secret, check=False).returncode == 4
    reply_to_reply = cli.run("comments", "reply", pid, "3", "x", token=secret, check=False)
    assert reply_to_reply.returncode == 1
    assert "first comment" in reply_to_reply.stderr
    assert cli.run("comments", "list", pid, check=False).returncode == 3  # needs a login
    missing = cli.run("comments", "list", "abcdefghijkm", token=secret, check=False)
    assert missing.returncode == 4
    blank = cli.run("comments", "add", pid, "-", token=secret, input="  \n", check=False)
    assert blank.returncode == 2


def test_data_commands(cli: Cli, server: Server, user: str, tmp_path: Path):
    page = tmp_path / "board.html"
    page.write_text("<title>Board</title><h1>Board</h1>")
    pid = cli.json("publish", str(page), token=user)["parcel"]["id"]
    assert cli.run("data", "list", pid, token=user).stdout.startswith(f"No data on {pid} yet.\n")

    # Inline, @file and stdin values; --string stores text as a JSON string.
    out = cli.run("data", "set", pid, "metrics", "latest", '{"p99_ms": 480, "ok": true}', token=user)
    assert out.stdout.startswith(f"Set metrics/latest on {pid} (")
    targets = tmp_path / "targets.json"
    targets.write_text('{\n  "p99_ms": 400\n}\n')
    cli.run("data", "set", pid, "metrics", "targets", f"@{targets}", token=user)
    cli.run("data", "set", pid, "metrics", "count", "-", token=user, input="3\n")
    cli.run("data", "set", pid, "notes", "intro", "-", "--string", token=user, input="Shipped *v2*.\n")

    def get(collection: str, key: str):
        return json.loads(cli.run("data", "get", pid, collection, key, token=user).stdout)

    assert get("metrics", "latest") == {"p99_ms": 480, "ok": True}
    assert get("metrics", "targets") == {"p99_ms": 400}
    assert get("metrics", "count") == 3
    assert get("notes", "intro") == "Shipped *v2*."
    pretty = cli.run("data", "get", pid, "metrics", "targets", token=user).stdout
    assert pretty == '{\n  "p99_ms": 400\n}\n'

    # Merge patches, inline and from stdin; null removes a field.
    updated = cli.json(
        "data", "update", pid, "metrics", "latest", '{"p99_ms": 412, "ok": null, "p50_ms": 90}',
        token=user,
    )
    assert updated["value"] == {"p99_ms": 412, "p50_ms": 90}
    cli.run("data", "update", pid, "metrics", "targets", "-", token=user, input='{"p50_ms": 80}')
    assert get("metrics", "targets") == {"p99_ms": 400, "p50_ms": 80}
    not_object = cli.run("data", "update", pid, "metrics", "latest", "[1]", token=user, check=False)
    assert not_object.returncode == 2

    summary = cli.json("data", "list", pid, token=user)
    assert {c["name"]: c["count"] for c in summary["collections"]} == {"metrics": 3, "notes": 1}
    assert summary["writable"] is True
    human = cli.run("data", "list", pid, token=user).stdout
    assert human.startswith("COLLECTION") and "metrics" in human and "data limit." in human
    items = cli.json("data", "list", pid, "metrics", token=user)
    assert [i["key"] for i in items["items"]] == ["count", "latest", "targets"]
    rows = cli.run("data", "list", pid, "metrics", token=user).stdout.splitlines()
    assert rows[0].startswith("KEY") and rows[2].startswith("latest")

    # The page reads what the CLI wrote.
    cli.run("update", pid, "--public", token=user)
    grant = httpx.get(f"{server.app_url}/api/v1/parcels/{pid}/view").json()
    on_page = httpx.get(f"{server.content_url}/d/{content_token(grant['content_url'])}/metrics/latest")
    assert on_page.json()["value"] == {"p99_ms": 412, "p50_ms": 90}

    # Visitors read; only the owner writes.
    assert json.loads(cli.run("data", "get", pid, "metrics", "count").stdout) == 3
    assert "Read-only" in cli.run("data", "list", pid).stdout
    denied = cli.run(
        "data", "set", pid, "metrics", "count", "4", token=server.issuer.token(), check=False
    )
    assert denied.returncode == 1 and "only the owner" in denied.stderr
    assert cli.run("data", "set", pid, "metrics", "count", "4", check=False).returncode == 3

    # Limits come from the server; names and JSON are checked before sending.
    big = json.dumps("x" * MAX_DATA_VALUE_BYTES)
    too_big = cli.run("data", "set", pid, "metrics", "big", big, token=user, check=False)
    assert too_big.returncode == 1 and "at most" in too_big.stderr
    bare = cli.run("data", "set", pid, "metrics", "x", "hello", token=user, check=False)
    assert bare.returncode == 2 and "--string" in bare.stderr
    assert cli.run("data", "get", pid, "bad name", "x", token=user, check=False).returncode == 2

    deleted = cli.run("data", "delete", pid, "metrics", "count", token=user).stdout
    assert deleted == f"Deleted metrics/count from {pid}\n"
    assert cli.run("data", "get", pid, "metrics", "count", token=user, check=False).returncode == 4
    assert cli.run("data", "clear", pid, "metrics", token=user).stdout == f"Cleared metrics on {pid}\n"
    assert cli.json("data", "list", pid, "metrics", token=user) == {"items": []}
