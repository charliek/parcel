"""The `parcel` CLI against the real server: login, publish, read, manage."""

from __future__ import annotations

import json
import os
import re
import subprocess
import time
from pathlib import Path

import httpx

from conftest import CLI_BIN, MAX_DATA_VALUE_BYTES, ROOT, Browser, Cli, Server, content_token

PAGE = """<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>CLI report</title>
<link rel="stylesheet" href="assets/site.css"></head>
<body><h1>Numbers</h1><script src="assets/app.js"></script></body></html>
"""


def device_login(cli: Cli, server: Server, approver: Browser) -> None:
    """Run `parcel login` for real and approve it the way the web app does:
    from the approver's signed-in browser."""
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
    assert approver.get(f"/api/v1/cli/device/{code}").json()["user_code"] == code
    assert approver.post(f"/api/v1/cli/device/{code}/approve").status_code == 204
    stdout, stderr = process.communicate(timeout=30)
    assert process.returncode == 0, stderr
    assert "Logged in" in stdout + stderr


def test_device_login_whoami_and_logout(cli: Cli, server: Server, people):
    approver = people("CLI User")
    assert cli.run("whoami", check=False).returncode == 3

    device_login(cli, server, approver)
    me = cli.json("whoami")
    assert me["email"] == approver.email

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


def test_comments_feedback_loop(cli: Cli, server: Server, user: str, people, tmp_path: Path):
    """What the skill teaches: list the feedback, fix, republish, resolve with a note."""
    # The owner's CLI holds a pcl_ token, as after `parcel login`.
    secret = user
    page = tmp_path / "index.html"
    page.write_text(PAGE)
    created = cli.json("publish", str(page), "--public", token=secret)
    pid, url = created["parcel"]["id"], created["parcel"]["url"]

    # A reviewer comments in the web viewer, pinned to the heading.
    anchor = {"path": "index.html", "selector": "html > body > h1", "text": "Numbers"}
    reviewer = people("Rita Reviewer")
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


def test_data_commands(
    cli: Cli, server: Server, user: str, stranger: Browser, tmp_path: Path
):
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
        "data", "set", pid, "metrics", "count", "4", token=stranger.token(), check=False
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


# ---------------------------------------------------------------- tags, slugs, search, pages


def git(repo: Path, *args: str) -> None:
    """`git` with no user or system configuration."""
    subprocess.run(
        [
            "git", "-C", str(repo), "-c", "user.name=Test", "-c", "user.email=t@example.com",
            "-c", "init.defaultBranch=main", *args,
        ],
        env={"PATH": os.environ["PATH"], "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1"},
        capture_output=True,
        check=True,
    )


def test_publish_from_a_git_repo_tags_the_project(cli: Cli, user: str, tmp_path: Path):
    repo = tmp_path / "funcproj-checkout"
    repo.mkdir()
    git(repo, "init", "-q")
    git(repo, "commit", "-q", "--allow-empty", "-m", "init")
    git(repo, "remote", "add", "origin", "https://ada:CANARY-7731@git.example.com/acme/FuncProj.git")
    page = tmp_path / "scratch" / "plan.html"
    page.parent.mkdir()
    page.write_text("<title>Func plan</title><p>x</p>")

    created = cli.json("publish", str(page), "--kind", "plan", token=user, cwd=repo)
    parcel = created["parcel"]
    assert parcel["tags"] == ["kind:plan", "project:funcproj"]
    assert created["created"] is True
    source = created["version"]["source"]
    assert source["repo"] == "git.example.com/acme/FuncProj"
    assert source["branch"] == "main" and len(source["commit"]) == 12
    assert source["client"].startswith("parcel-cli/")
    info = cli.run("info", parcel["id"], token=user).stdout
    assert "source:      git.example.com/acme/FuncProj, branch main" in info
    assert "CANARY" not in json.dumps(created) + info

    # Suppressed, and outside any repository.
    bare = cli.json("publish", str(page), "--no-auto-tags", "--no-source", token=user, cwd=repo)
    assert bare["parcel"]["tags"] == [] and "source" not in bare["version"]
    outside = cli.json("publish", str(page), token=user, cwd=tmp_path)
    assert outside["parcel"]["tags"] == []
    assert set(outside["version"]["source"]) <= {"host", "client"}


def test_slug_upserts_and_slug_refs(cli: Cli, user: str, tmp_path: Path):
    page = tmp_path / "plan.html"
    page.write_text("<title>Upserted plan</title><p>one</p>")
    first = cli.json("publish", str(page), "--slug", "func/plan-1", token=user)
    page.write_text("<title>Upserted plan</title><p>two</p>")
    second = cli.json(
        "publish", str(page), "--slug", "func/plan-1", "--base-version", "1", token=user
    )
    pid = first["parcel"]["id"]
    assert first["created"] is True and second["created"] is False
    assert second["parcel"]["id"] == pid and second["version"]["version"] == 2
    assert [v["version"] for v in cli.json("versions", "func/plan-1", token=user)["versions"]] == [2, 1]
    stale = cli.run(
        "publish", str(page), "--slug", "func/plan-1", "--base-version", "1", token=user, check=False
    )
    assert stale.returncode == 1

    handle = cli.json("handle", token=user)["handle"]
    assert cli.run("cat", f"@{handle}/func/plan-1", token=user).stdout.endswith("<p>two</p>")
    human = cli.run("publish", str(page), "--slug", "func/plan-1", token=user).stdout
    assert human.startswith('Updated "Upserted plan"'), human
    assert f"/@{handle}/func/plan-1" in human


def test_list_filters_and_full_text_snippets(cli: Cli, user: str, tmp_path: Path):
    for name, body, extra in [
        ("alpha", "The zygomorphic flowers bloom in spring.", ["--project", "fx", "--kind", "plan"]),
        ("beta", "Nothing to see here.", ["--tag", "misc"]),
    ]:
        page = tmp_path / f"{name}.html"
        page.write_text(f"<title>{name} page</title><p>{body}</p>")
        cli.run("publish", str(page), *extra, token=user, cwd=tmp_path)

    by_project = cli.json("list", "--project", "fx", "--kind", "plan", token=user)
    assert [p["title"] for p in by_project["parcels"]] == ["alpha page"]
    assert [p["title"] for p in cli.json("list", "--tag", "misc", token=user)["parcels"]] == ["beta page"]
    human = cli.run("list", "--tag", "misc", token=user).stdout
    assert "TAGS" in human.splitlines()[0] and "misc" in human

    # Indexing runs after the publish commits: wait for it.
    deadline = time.monotonic() + 10
    while True:
        out = cli.run("list", "-q", "zygomorphic", token=user).stdout
        if "**zygomorphic**" in out or time.monotonic() > deadline:
            break
        time.sleep(0.2)
    lines = out.splitlines()
    assert "alpha page" in lines[1], out
    assert lines[2].startswith("    … ") and "**zygomorphic**" in lines[2], out
    assert cli.json("list", "-q", "zygomorphic", token=user)["parcels"][0]["match"]["segments"]


def test_tags_archive_link_and_handle(cli: Cli, server: Server, user: str, tmp_path: Path):
    page = tmp_path / "index.html"
    page.write_text("<title>Report [draft] (v1)</title><a href='b.html'>b</a>")
    second = tmp_path / "b.html"
    second.write_text("<title>Second page</title><p>b</p>")
    site = tmp_path / "site"
    site.mkdir()
    page.rename(site / "index.html")
    second.rename(site / "b.html")
    created = cli.json("publish", str(site), "--tag", "Old_Name", token=user, cwd=tmp_path)
    pid = created["parcel"]["id"]

    tags = cli.json("tags", token=user)
    assert tags == {"tags": [{"tag": "old-name", "count": 1}], "total": 1}
    renamed = cli.json("tags", "rename", "old-name", "new-name", token=user)
    assert renamed == {"updated": 1}
    assert "new-name" in cli.run("tags", token=user).stdout
    tagged = cli.json("tag", pid, "--add", "extra", "--remove", "new-name", token=user)
    assert tagged["tags"] == ["extra"]
    assert cli.json("tags", "delete", "extra", token=user) == {"updated": 1}

    archived = cli.json("archive", pid, token=user)
    assert archived["archived_at"]
    assert cli.json("list", token=user)["parcels"] == []
    assert [p["id"] for p in cli.json("list", "--archived-only", token=user)["parcels"]] == [pid]
    assert cli.json("unarchive", pid, token=user)["archived_at"] is None

    cli.run("update", pid, "--slug", "func/report", token=user)
    handle = cli.json("handle", token=user)["handle"]
    clean = f"{server.app_url}/@{handle}/func/report"
    assert cli.run("link", pid, token=user).stdout == f"{clean}\n"
    assert cli.run("link", pid, "--markdown", token=user).stdout == (
        f"[Report \\[draft\\] \\(v1\\)]({clean})\n"
    )
    deep = cli.json("link", pid, "--page", "b.html", token=user)
    assert deep["canonical_url"] == f"{clean}?page=b.html"
    assert deep["url"] == f"{server.app_url}/p/{pid}?page=b.html"
    # The deep link's page is one the viewer can open.
    assert httpx.get(
        f"{server.app_url}/api/v1/parcels/{pid}/view", params={"path": "b.html"},
        headers={"Authorization": f"Bearer {user}"},
    ).status_code == 200
    assert cli.run("link", pid, "--page", "nope.html", token=user, check=False).returncode == 4
    info = cli.run("info", f"@{handle}/func/report", token=user).stdout
    assert "Pages in v1" in info and "Second page" in info

    new_handle = f"h{handle[:20]}x"
    changed = cli.json("handle", new_handle, token=user)
    assert changed == {"handle": new_handle, "previous": handle}
    # The old handle still resolves.
    assert cli.json("info", f"@{handle}/func/report", token=user)["parcel"]["id"] == pid
