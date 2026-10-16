"""Visibility levels and sharing with named people, through the API, the
content origin and the CLI."""

from __future__ import annotations

from pathlib import Path

import httpx

from conftest import Browser, Cli, Server, content_token, unique_email
from test_api import PAGE, publish

CORS = {"Origin": "null"}  # what a sandboxed page's fetches carry


def test_sharing_with_someone_before_they_sign_in(
    server: Server, api: Browser, stranger: Browser, people
):
    pid = publish(api, {"index.html": PAGE}).json()["parcel"]["id"]
    shares = f"/api/v1/parcels/{pid}/shares"
    comments = f"/api/v1/parcels/{pid}/comments"
    data = f"/api/v1/parcels/{pid}/data"
    bea_email = unique_email("Bea Shared")

    # Bea has never signed in; sharing by email works anyway (any case).
    put = api.put(f"{shares}/{bea_email.upper()}", json={"role": "viewer"})
    assert put.status_code == 200, put.text
    share = put.json()
    assert (share["email"], share["role"]) == (bea_email, "viewer")
    assert "name" not in share
    assert api.get(shares).json()["shares"] == [share]
    assert api.put(f"{shares}/{bea_email}", json={}).json()["role"] == "viewer"  # the default

    bea = people("Bea Shared", email=bea_email)
    listed = bea.get("/api/v1/shared").json()
    assert listed["total"] == 1
    entry = listed["parcels"][0]
    assert (entry["parcel"]["id"], entry["role"]) == (pid, "viewer")
    assert entry["owner_name"] == "Functional Tester"
    detail = bea.get(f"/api/v1/parcels/{pid}").json()
    assert (detail["is_owner"], detail["access"]) == (False, "viewer")
    assert detail["owner_name"] == "Functional Tester"
    # Shared parcels aren't hers: her own list stays empty.
    assert bea.get("/api/v1/parcels").json()["total"] == 0
    # The owner's list now knows who she is.
    assert api.get(shares).json()["shares"][0]["name"] == "Bea Shared"

    grant = bea.get(f"/api/v1/parcels/{pid}/view").json()
    assert (grant["access"], grant["owner_name"]) == ("viewer", "Functional Tester")
    bea_data = f"{server.content_url}/d/{content_token(grant['content_url'])}"
    assert api.post(comments, json={"body": "Owner's note"}).status_code == 201
    assert api.put(f"{data}/votes/a", json=1).status_code == 200
    with server.content() as content:
        assert content.get(grant["content_url"]).status_code == 200
        # A viewer reads comments and data, and writes neither.
        assert [c["body"] for c in bea.get(comments).json()["comments"]] == ["Owner's note"]
        refused = bea.post(comments, json={"body": "Can I?"})
        assert refused.status_code == 403, refused.text
        assert bea.get(f"{data}/votes/a").json()["value"] == 1
        assert bea.put(f"{data}/votes/b", json=2).status_code == 403
        assert content.get(f"{bea_data}/votes/a").json()["value"] == 1
        assert content.get(bea_data).json()["writable"] is False
        assert content.put(f"{bea_data}/votes/a", json=5, headers=CORS).status_code == 403
        # Only the owner shares or changes it (403: she can see it).
        assert bea.get(shares).status_code == 403
        assert bea.put(f"{shares}/{unique_email('Her Friend')}", json={}).status_code == 403
        assert bea.patch(f"/api/v1/parcels/{pid}", json={"title": "x"}).status_code == 403
        # Nobody else signed in can see it, and can't tell it exists.
        assert stranger.get(f"/api/v1/parcels/{pid}").status_code == 404
        assert stranger.get(shares).status_code == 404

        # A commenter also comments.
        promoted = api.put(f"{shares}/{bea_email}", json={"role": "commenter"})
        assert promoted.json()["role"] == "commenter"
        assert bea.get(f"/api/v1/parcels/{pid}").json()["access"] == "commenter"
        reply = bea.post(comments, json={"body": "Now I can", "reply_to": 1})
        assert reply.status_code == 201, reply.text
        assert reply.json()["replies"][0]["author"] == {
            "name": "Bea Shared",
            "is_owner": False,
            "is_you": True,
        }
        assert bea.get("/api/v1/shared").json()["parcels"][0]["role"] == "commenter"

        # Unsharing cuts off the page she has open, at once.
        assert api.delete(f"{shares}/{bea_email}").status_code == 204
        assert content.get(grant["content_url"]).status_code == 403
        assert content.get(f"{bea_data}/votes/a").status_code == 403
        assert bea.get(f"/api/v1/parcels/{pid}").status_code == 404
        assert bea.get(f"/api/v1/parcels/{pid}/view").status_code == 404
        assert bea.get(comments).status_code == 404
        assert bea.get("/api/v1/shared").json() == {"parcels": [], "total": 0}
    assert api.delete(f"{shares}/{bea_email}").status_code == 404
    assert api.get(shares).json() == {"shares": []}


def test_share_requests_are_checked(api: Browser):
    pid = publish(api, {"index.html": PAGE}).json()["parcel"]["id"]
    shares = f"/api/v1/parcels/{pid}/shares"
    for bad in ["not-an-email", "a@localhost", "a b@example.com"]:
        refused = api.put(f"{shares}/{bad}", json={"role": "viewer"})
        assert refused.status_code == 400, (bad, refused.text)
        assert "is not an email address" in refused.json()["error"]["message"]
    yourself = api.put(f"{shares}/{api.email}", json={"role": "viewer"})
    assert yourself.status_code == 400
    assert "owners always have access" in yourself.json()["error"]["message"]
    assert api.put(f"{shares}/{unique_email('E')}", json={"role": "editor"}).status_code == 422
    assert api.get("/api/v1/parcels/abcdefghijkm/shares").status_code == 404
    assert api.get(shares).json() == {"shares": []}


def test_internal_parcels_open_to_everyone_signed_in(
    server: Server, api: Browser, stranger: Browser
):
    created = publish(api, {"index.html": PAGE}, {"visibility": "internal"})
    assert created.status_code == 201, created.text
    parcel = created.json()["parcel"]
    pid = parcel["id"]
    assert parcel["visibility"] == "internal"
    comments = f"/api/v1/parcels/{pid}/comments"
    data = f"/api/v1/parcels/{pid}/data"
    assert api.put(f"{data}/notes/intro", json="hello").status_code == 200

    # Anyone signed in views and comments.
    detail = stranger.get(f"/api/v1/parcels/{pid}").json()
    assert (detail["is_owner"], detail["access"]) == (False, "commenter")
    assert detail["owner_name"] == "Functional Tester"
    grant = stranger.get(f"/api/v1/parcels/{pid}/view").json()
    assert grant["access"] == "commenter"
    with server.content() as content:
        page = content.get(grant["content_url"])
        assert page.status_code == 200
        assert "<h1>Hello</h1>" in page.text
        made = stranger.post(comments, json={"body": "Seen it"})
        assert made.status_code == 201, made.text
        assert stranger.get(f"{data}/notes/intro").json()["value"] == "hello"
        assert stranger.put(f"{data}/notes/intro", json="mine").status_code == 403
        # Internal parcels aren't "shared with" anyone, nor anyone's own.
        assert stranger.get("/api/v1/shared").json()["total"] == 0
        assert stranger.get("/api/v1/parcels").json()["total"] == 0

        # Signed out, it doesn't exist.
        with server.client() as anonymous:
            assert anonymous.get(f"/api/v1/parcels/{pid}").status_code == 404
            assert anonymous.get(f"/api/v1/parcels/{pid}/view").status_code == 404
            assert anonymous.get(data).status_code == 404
            assert anonymous.get(f"/api/v1/parcels/{pid}/versions/latest/files/index.html").status_code == 404

        # Going private closes it again, open pages included.
        assert api.patch(f"/api/v1/parcels/{pid}", json={"visibility": "private"}).status_code == 200
        assert content.get(grant["content_url"]).status_code == 403
        assert stranger.get(f"/api/v1/parcels/{pid}").status_code == 404
        assert stranger.post(comments, json={"body": "again"}).status_code == 404


def test_public_links_can_be_turned_off(
    server: Server, no_public_server: Server, tmp_path: Path
):
    """`PARCEL_ALLOW_PUBLIC=false`: public can't be set, and parcels made
    public before act (and report) as internal — even pages already open."""
    off = no_public_server
    info = httpx.get(f"{off.app_url}/api/v1/info").json()
    assert info["allow_public"] is False

    email = unique_email("Ola Owner")
    owner = off.sign_in(email, name="Ola Owner")
    refused = publish(owner, {"index.html": PAGE}, {"visibility": "public"})
    assert refused.status_code == 400
    message = refused.json()["error"]["message"]
    assert message.startswith("public links are turned off on this server"), message
    pid = publish(owner, {"index.html": PAGE}, {"visibility": "internal"}).json()["parcel"]["id"]
    assert owner.patch(f"/api/v1/parcels/{pid}", json={"visibility": "public"}).status_code == 400

    # The other server (same database) still allows public links.
    elsewhere = server.sign_in(email)
    assert elsewhere.patch(f"/api/v1/parcels/{pid}", json={"visibility": "public"}).status_code == 200
    public_grant = httpx.get(f"{server.app_url}/api/v1/parcels/{pid}/view").json()
    assert public_grant["access"] == "public"
    with server.content() as content:
        assert content.get(public_grant["content_url"]).status_code == 200
        # Here it is internal: hidden from anonymous callers, open to anyone
        # signed in, and the public page link stops working.
        assert httpx.get(f"{off.app_url}/api/v1/parcels/{pid}").status_code == 404
        assert owner.get(f"/api/v1/parcels/{pid}").json()["parcel"]["visibility"] == "internal"
        assert owner.get("/api/v1/parcels").json()["parcels"][0]["visibility"] == "internal"
        on_off = public_grant["content_url"].replace(server.content_url, off.content_url)
        assert content.get(on_off).status_code == 403
    someone = off.sign_in(unique_email("Sid"))
    assert someone.get(f"/api/v1/parcels/{pid}").json()["access"] == "commenter"
    someone.close()

    # The CLI: `update --public` shows the server's words; `publish --public`
    # is refused before uploading.
    home = tmp_path / "home"
    home.mkdir()
    cli = Cli(server=off, home=home)
    token = owner.token()
    updated = cli.run("update", pid, "--public", token=token, check=False)
    assert updated.returncode == 1
    assert updated.stderr == f"error: {message}\n"
    page = tmp_path / "page.html"
    page.write_text(PAGE)
    published = cli.run("publish", str(page), "--public", token=token, check=False)
    assert published.returncode == 1
    assert "public links are turned off on" in published.stderr
    assert "--internal" in published.stderr
    assert cli.json("publish", str(page), "--internal", token=token)["parcel"]["visibility"] == "internal"
    owner.close()
    elsewhere.close()


def test_sharing_from_the_cli(
    cli: Cli, server: Server, user: str, stranger: Browser, people, tmp_path: Path
):
    page = tmp_path / "report.html"
    page.write_text("<title>Shared report</title><h1>Numbers</h1>")
    pid = cli.json("publish", str(page), token=user)["parcel"]["id"]
    bea_email = unique_email("Bea Cli")

    out = cli.run("share", pid, bea_email.upper(), "--role", "commenter", token=user).stdout
    assert out.startswith(
        f"Shared {pid} with {bea_email} (commenter)\n\nShared with 1 person:\nEMAIL"
    ), out
    assert "not signed in yet" in out
    assert [s["email"] for s in cli.json("shares", pid, token=user)["shares"]] == [bea_email]

    bea = people("Bea Cli", email=bea_email)
    bea_token = bea.token()
    assert "Bea Cli" in cli.run("shares", pid, token=user).stdout

    listed = cli.run("list", "--shared", token=bea_token).stdout.splitlines()
    assert listed[0].split() == ["ID", "TITLE", "OWNER", "ROLE", "VERSION", "UPDATED", "COMMENTS"]
    for cell in [pid, "Shared report", "Functional Tester", "commenter", "v1"]:
        assert cell in listed[1], listed
    assert cli.json("list", "--shared", token=bea_token)["total"] == 1
    info = cli.run("info", pid, token=bea_token).stdout
    assert "owner:       Functional Tester" in info
    assert "access:      commenter (you can view and comment)" in info
    assert cli.run("comments", "add", pid, "Looks right", token=bea_token).stdout.startswith(
        "Added comment #1"
    )
    assert cli.run("cat", pid, token=bea_token).stdout == page.read_text()

    # Only the owner shares.
    theirs = cli.run("share", pid, unique_email("Friend"), token=bea_token, check=False)
    assert theirs.returncode == 1 and "only the owner" in theirs.stderr
    # Malformed addresses never reach the server.
    assert cli.run("share", pid, "bea", token=user, check=False).returncode == 2

    gone = cli.run("unshare", pid, bea_email, "nobody@example.com", token=user)
    assert gone.stdout == (
        f"Unshared {pid} with {bea_email}\n\n"
        f"{pid} is not shared with anyone. Share it with `parcel share {pid} <email>`.\n"
    )
    assert gone.stderr == f"note: {pid} was not shared with nobody@example.com\n"
    assert cli.run("info", pid, token=bea_token, check=False).returncode == 4
    assert cli.run("list", "--shared", token=bea_token).stdout == (
        "Nothing has been shared with you yet.\n"
    )

    # Internal: anyone signed in, no share needed.
    assert cli.json("update", pid, "--internal", token=user)["visibility"] == "internal"
    seen = cli.run("info", pid, token=stranger.token()).stdout
    assert "visibility:  internal" in seen and "access:      commenter" in seen
    assert cli.run("info", pid, check=False).returncode == 4  # signed out


def test_a_paused_token_from_the_cli(cli: Cli, server: Server, people, tmp_path: Path):
    person = people("Pia Paused")
    token = person.token()
    assert cli.json("whoami", token=token)["email"] == person.email
    server.sql(
        "UPDATE users SET last_login_at = now() - interval '45 days' "
        f"WHERE email = '{person.email}'"
    )
    paused = cli.run("list", token=token, check=False)
    assert paused.returncode == 3
    # The server's instruction, as written, and not a contradicting hint.
    assert paused.stderr.startswith(
        f"error: this token is paused: sign in to Parcel on the web ({server.app_url})"
    ), paused.stderr
    assert "parcel login" not in paused.stderr
    server.sign_in(person.email, browser=person)
    assert cli.run("list", token=token).returncode == 0
