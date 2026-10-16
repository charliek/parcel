"""Signing in through OIDC providers, sessions, CSRF, logout and API tokens."""

from __future__ import annotations

import base64
import hashlib

import httpx

from conftest import (
    TOKEN_LOGIN_MAX_AGE_DAYS,
    Browser,
    Server,
    query_of,
    signin_error,
    unique_email,
)


def test_sign_in_through_the_provider(server: Server):
    info = httpx.get(f"{server.app_url}/api/v1/info").json()
    assert info["auth"] == {
        "mode": "oidc",
        "providers": [
            {"id": "test", "label": "Test IdP"},
            {"id": "corp", "label": "Corp SSO"},
        ],
    }
    assert info["allow_public"] is True

    email = unique_email("Ada Lovelace")
    server.idp.profile(email, name="Ada Lovelace")
    browser = Browser(server, email)

    # Leg one: the server sends the browser to the provider with PKCE and a
    # nonce, and remembers them in a short-lived cookie scoped to /auth.
    login = browser.get(
        "/auth/login",
        params={"provider": "test", "return_to": "/p/abcdefghijkm?v=2", "login_hint": email},
    )
    assert login.status_code == 303
    assert login.headers["cache-control"] == "no-store"
    authorize = login.headers["location"]
    tenant = server.idp.tenants["test"]
    assert authorize.startswith(f"{tenant.issuer}/authorize?")
    q = query_of(authorize)
    assert q["response_type"] == "code"
    assert q["client_id"] == "parcel-functional"
    assert q["redirect_uri"] == f"{server.app_url}/auth/callback"
    assert q["scope"].split() == ["openid", "email", "profile"]
    assert q["code_challenge_method"] == "S256"
    assert q["login_hint"] == email
    assert len(q["state"]) >= 32 and len(q["nonce"]) >= 32 and q["state"] != q["nonce"]
    login_cookie = login.headers["set-cookie"]
    assert login_cookie.startswith("parcel_login=")
    for attribute in ("HttpOnly", "SameSite=Lax", "Path=/;"):
        assert attribute in login_cookie, login_cookie

    # Leg two: the provider redirects back; the server redeems the code and
    # signs the browser in.
    at_provider = httpx.get(authorize)
    assert at_provider.status_code == 302
    callback = browser.get(at_provider.headers["location"])
    assert callback.status_code == 303
    assert callback.headers["location"] == "/p/abcdefghijkm?v=2"
    session_cookie = next(
        c for c in callback.headers.get_list("set-cookie") if c.startswith("parcel_session=")
    )
    for attribute in ("HttpOnly", "SameSite=Lax", "Path=/"):
        assert attribute in session_cookie, session_cookie
    assert "parcel_session" in browser.cookies

    # The code went to the token endpoint with the verifier for the challenge,
    # as a public client (client_id in the form, no secret).
    redeemed = server.idp.token_requests[-1]
    assert redeemed["_tenant"] == "test" and redeemed["_authorization"] is None
    assert redeemed["client_id"] == "parcel-functional"
    assert redeemed["redirect_uri"] == f"{server.app_url}/auth/callback"
    digest = hashlib.sha256(redeemed["code_verifier"].encode()).digest()
    assert base64.urlsafe_b64encode(digest).rstrip(b"=").decode() == q["code_challenge"]

    me = browser.me()
    assert me.status_code == 200, me.text
    assert (me.json()["email"], me.json()["name"]) == (email, "Ada Lovelace")
    assert me.json()["signed_in_with"] == "Test IdP"

    # A code is good once; replaying the callback finds no sign-in in progress.
    replay = browser.get(at_provider.headers["location"])
    assert signin_error(replay) == "expired"
    browser.close()


def test_unsafe_return_to_lands_home(server: Server):
    email = unique_email("Rhea")
    browser = server.sign_in(email, return_to="/")
    answer = server.begin_sign_in(Browser(server, email), email, return_to="//evil.example/x")
    assert answer.headers["location"] == "/"
    browser.close()


def test_a_confidential_client_with_provider_logout(server: Server):
    email = unique_email("Cora", domain="corp.example")
    browser = server.sign_in(email, provider="corp")
    redeemed = server.idp.token_requests[-1]
    # A client secret means HTTP Basic auth, and no secret in the form.
    assert redeemed["_tenant"] == "corp"
    assert redeemed["_authorization"].startswith("Basic ")
    assert "client_secret" not in redeemed
    assert browser.me().json()["signed_in_with"] == "Corp SSO"

    out = browser.post("/auth/logout")
    assert out.status_code == 200, out.text
    redirect = out.json()["redirect"]
    tenant = server.idp.tenants["corp"]
    assert redirect.startswith(f"{tenant.issuer}/logout?")
    q = query_of(redirect)
    assert q["client_id"] == "parcel-corp"
    assert q["post_logout_redirect_uri"] == f"{server.app_url}/"
    assert q["id_token_hint"].count(".") == 2
    assert browser.me().status_code == 401
    browser.close()


def test_one_person_across_providers(server: Server):
    """A new identity with a known email is the same user (shares and
    parcels follow the person, not the provider)."""
    email = unique_email("Lin", domain="corp.example")
    via_corp = server.sign_in(email, provider="corp", name="Lin")
    created = via_corp.post(
        "/api/v1/parcels",
        files=[("file", ("index.html", b"<title>Lin's</title>", "text/html"))],
    )
    assert created.status_code == 201, created.text
    via_test = server.sign_in(email, provider="test")
    listed = via_test.get("/api/v1/parcels").json()["parcels"]
    assert [p["id"] for p in listed] == [created.json()["parcel"]["id"]]
    assert via_test.me().json()["signed_in_with"] == "Test IdP"
    via_corp.close()
    via_test.close()


def test_identity_claims_from_userinfo(server: Server):
    """A provider whose ID token carries only `sub`: email and name come from
    userinfo."""
    email = unique_email("Uma")
    browser = server.sign_in(email, name="Uma Userinfo", email_in_id_token=False)
    me = browser.me().json()
    assert (me["email"], me["name"]) == (email, "Uma Userinfo")
    browser.close()


def test_refusals_land_on_the_sign_in_page(server: Server):
    exchange_errors = len(server.idp.token_errors)

    def refused(address: str, provider: str = "test", **profile) -> str | None:
        if profile:
            server.idp.profile(address, provider, **profile)
        with Browser(server, address) as browser:
            answer = server.begin_sign_in(browser, address, provider, return_to="/tokens")
            assert "parcel_session" not in browser.cookies
            assert browser.me().status_code == 401
            location = query_of(answer.headers["location"])
            assert location.get("return_to") == "/tokens", answer.headers["location"]
            return signin_error(answer)

    # corp signs in only corp.example addresses.
    assert refused(unique_email("Out"), provider="corp") == "domain"
    # An email the provider doesn't vouch for is no identity (shares go by email).
    assert refused(unique_email("Unv"), email_verified=False) == "unverified"
    assert refused(unique_email("Unv", "corp.example"), "corp", email_verified=False) == "unverified"
    assert refused(unique_email("No"), email="") == "unverified"
    # The person said no at the provider.
    assert refused(unique_email("Den"), deny=True) == "denied"
    # Each was the server's policy after a good code exchange, not a failed one.
    assert len(server.idp.token_errors) == exchange_errors

    # An unknown provider, or a callback this browser never started.
    with Browser(server, "x@example.com") as browser:
        unknown = browser.get("/auth/login", params={"provider": "nope", "return_to": "/x"})
        assert unknown.headers["location"].startswith("/signin?return_to=%2Fx")
        stray = browser.get("/auth/callback", params={"code": "c", "state": "s"})
        assert signin_error(stray) == "expired"

    # A state that doesn't match the sign-in this browser started.
    email = unique_email("State")
    with Browser(server, email) as browser:
        login = browser.get("/auth/login", params={"provider": "test", "login_hint": email})
        at_provider = httpx.get(login.headers["location"])
        forged = at_provider.headers["location"].replace("state=", "state=x")
        assert signin_error(browser.get(forged)) == "expired"


def test_cookie_writes_need_the_apps_origin(server: Server, owner: Browser):
    # The owner's browser sends Origin; a page elsewhere riding the cookie
    # can't make one (or makes the wrong one).
    assert owner.post("/api/v1/tokens", json={"name": "ok"}).status_code == 201
    with httpx.Client(base_url=server.app_url, cookies=owner.cookies) as no_origin:
        refused = no_origin.post("/api/v1/tokens", json={"name": "csrf"})
        assert refused.status_code == 403
        assert refused.json()["error"]["message"] == "cross-origin request refused"
        assert no_origin.get("/api/v1/me").status_code == 200  # reads don't need it
    wrong = owner.post(
        "/api/v1/tokens", json={"name": "csrf"}, headers={"Origin": "http://evil.example"}
    )
    assert wrong.status_code == 403
    # A cross-site request never counts as signed in, even to read.
    assert owner.get("/api/v1/me", headers={"Sec-Fetch-Site": "cross-site"}).status_code == 401
    assert owner.get("/api/v1/me", headers={"Sec-Fetch-Site": "same-site"}).status_code == 401
    assert owner.get("/api/v1/me", headers={"Sec-Fetch-Site": "same-origin"}).status_code == 200
    names = [t["name"] for t in owner.get("/api/v1/tokens").json()["tokens"]]
    assert "csrf" not in names

    # Bearer tokens need no Origin: the CLI and agents aren't browsers.
    secret = owner.token()
    with server.client(secret) as agent:
        assert agent.post("/api/v1/tokens", json={"name": "by-agent"}).status_code == 201


def test_api_bearers_are_parcel_tokens_only(server: Server, owner: Browser):
    # A perfectly good ID token from a configured provider is still not an
    # API credential.
    id_token = server.idp.id_token(sub="someone", email=owner.email, email_verified=True)
    with server.client(id_token) as jwt_bearer:
        refused = jwt_bearer.get("/api/v1/me")
    assert refused.status_code == 401
    assert "pcl_" in refused.json()["error"]["message"]
    with server.client("pcl_" + "x" * 32) as forged:
        assert forged.get("/api/v1/me").status_code == 401


def test_logout_and_logout_everywhere(server: Server):
    email = unique_email("Lou")
    laptop = server.sign_in(email)
    phone = server.sign_in(email)

    # Logout needs the app's Origin like any cookie write.
    with httpx.Client(base_url=server.app_url, cookies=laptop.cookies) as no_origin:
        assert no_origin.post("/auth/logout").status_code == 403

    out = laptop.post("/auth/logout")
    assert out.status_code == 200
    assert out.json() == {"redirect": "/"}
    assert "parcel_session" not in laptop.cookies
    assert laptop.me().status_code == 401
    assert phone.me().status_code == 200, "logging out one browser leaves the others"

    tablet = server.sign_in(email)
    everywhere = phone.post("/auth/logout", params={"everywhere": "true"})
    assert everywhere.status_code == 200
    assert phone.me().status_code == 401
    # Another browser's still-held cookie stopped working too, and the server
    # tells it to drop it.
    ended = tablet.me()
    assert ended.status_code == 401
    assert "session has ended" in ended.json()["error"]["message"]
    assert "parcel_session" not in tablet.cookies

    # Signing in again works as usual.
    again = server.sign_in(email, browser=laptop)
    assert again.me().status_code == 200
    for browser in (laptop, phone, tablet):
        browser.close()


def test_tokens_pause_without_a_recent_web_sign_in(server: Server, people):
    person = people("Pat Paused")
    secret = person.token()
    with server.client(secret) as agent:
        assert agent.get("/api/v1/me").status_code == 200

        server.sql(
            "UPDATE users SET last_login_at = now() - interval "
            f"'{TOKEN_LOGIN_MAX_AGE_DAYS + 1} days' WHERE email = '{person.email}'"
        )
        paused = agent.get("/api/v1/me")
        assert paused.status_code == 401
        message = paused.json()["error"]["message"]
        assert message.startswith("this token is paused: sign in to Parcel on the web")
        assert server.app_url in message and f"every {TOKEN_LOGIN_MAX_AGE_DAYS} days" in message
        # The web session isn't affected: signing in is how it's fixed.
        assert person.me().status_code == 200

        server.sign_in(person.email, browser=person)
        assert agent.get("/api/v1/me").status_code == 200, "a web sign-in un-pauses it"

        # An operator disabling the account stops sessions and tokens at once,
        # and a new sign-in too.
        server.sql(f"UPDATE users SET disabled_at = now() WHERE email = '{person.email}'")
        assert agent.get("/api/v1/me").status_code == 401
        assert person.me().status_code == 401
        with Browser(server, person.email) as fresh:
            assert signin_error(server.begin_sign_in(fresh, person.email)) == "disabled"
