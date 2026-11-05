"""Entra and Graph as outlook-sync meets them, behind nginx's TLS for their real host names.

It answers the sign-in, the token endpoint (checking the client assertion against the sync's
certificate and the code against its PKCE challenge), one subscription (validating its URLs
the way Graph does) and the default calendar's delta query, two occurrences a page. The test
drives it on /test/: the calendar's events, and notifications and lifecycle events to send.
"""

import base64
import hashlib
import json
import sys
import threading
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from cryptography import x509
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import padding
from cryptography.hazmat.primitives.serialization import Encoding

CERTIFICATE, LOG = sys.argv[1], sys.argv[2]
GRAPH = "https://graph.microsoft.com/v1.0"
PAGE = 2

lock = threading.Lock()
state = {
    "events": {},
    "versions": {},
    "removed": {},
    "version": 0,
    "codes": {},
    "refresh": None,
    "access": None,
    "subscription": None,
    "subscriptions": 0,
}


def log(kind, **fields):
    with open(LOG, "a") as f:
        f.write(json.dumps({"kind": kind, **fields}) + "\n")


def b64url(data):
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def unb64url(text):
    return base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))


def check_assertion(jwt, client_id):
    head, claims, signature = jwt.split(".")
    header, body = json.loads(unb64url(head)), json.loads(unb64url(claims))
    with open(CERTIFICATE, "rb") as f:
        cert = x509.load_pem_x509_certificate(f.read())
    der = cert.public_bytes(Encoding.DER)
    assert header["alg"] == "PS256", header
    assert header["x5t#S256"] == b64url(hashlib.sha256(der).digest()), header
    assert body["iss"] == body["sub"] == client_id, body
    assert body["aud"] == "https://login.microsoftonline.com/organizations/oauth2/v2.0/token", body
    cert.public_key().verify(
        unb64url(signature),
        f"{head}.{claims}".encode(),
        padding.PSS(mgf=padding.MGF1(hashes.SHA256()), salt_length=padding.PSS.AUTO),
        hashes.SHA256(),
    )


def post(url, body=None, query=None):
    if query:
        url += "?" + urllib.parse.urlencode(query)
    data = json.dumps(body).encode() if body is not None else b""
    request = urllib.request.Request(url, data=data, method="POST", headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=10) as reply:
            return reply.status, reply.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


class Handler(BaseHTTPRequestHandler):
    def reply(self, status, body=None, headers=()):
        payload = b"" if body is None else (body if isinstance(body, bytes) else json.dumps(body).encode())
        self.send_response(status)
        for name, value in headers:
            self.send_header(name, value)
        if body is not None and not isinstance(body, bytes):
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def body(self):
        return self.rfile.read(int(self.headers.get("Content-Length") or 0))

    def authorized(self):
        ok = self.headers.get("Authorization") == f"Bearer {state['access']}"
        if not ok:
            self.reply(401, {"error": {"code": "InvalidAuthenticationToken", "message": "no"}})
        return ok

    def do_GET(self):
        url = urllib.parse.urlparse(self.path)
        query = dict(urllib.parse.parse_qsl(url.query))
        if url.path == "/organizations/oauth2/v2.0/authorize":
            code = b64url(hashlib.sha256(query["state"].encode()).digest())
            with lock:
                state["codes"][code] = (query["code_challenge"], query["redirect_uri"], query["client_id"])
            log("authorize", scope=query["scope"], method=query["code_challenge_method"])
            to = query["redirect_uri"] + "?" + urllib.parse.urlencode({"code": code, "state": query["state"]})
            return self.reply(302, b"", [("Location", to)])
        if url.path == "/v1.0/me/calendarView/delta" and self.authorized():
            return self.delta(query)
        if url.path == "/test/state":
            with lock:
                return self.reply(200, {k: state[k] for k in ("subscription", "subscriptions", "version")})
        self.reply(404, {"error": {"code": "NotFound", "message": url.path}})

    def delta(self, query):
        with lock:
            if "$deltatoken" in query:
                since = int(query["$deltatoken"])
                changed = [state["events"][i] for i, v in state["versions"].items() if v > since and i in state["events"]]
                removed = [{"id": i, "@removed": {"reason": "deleted"}} for i, v in state["removed"].items() if v > since]
                log("delta", since=since, changed=len(changed), removed=len(removed))
                items = changed + removed
            else:
                log("listing", start=query.get("startDateTime"), end=query.get("endDateTime"), skip=query.get("$skiptoken"))
                items = sorted(state["events"].values(), key=lambda e: e["id"])
            skip = int(query.get("$skiptoken", 0))
            page, rest = items[skip : skip + PAGE], items[skip + PAGE :]
            body = {"value": page}
            if rest:
                more = dict(query, **{"$skiptoken": str(skip + PAGE)})
                body["@odata.nextLink"] = f"{GRAPH}/me/calendarView/delta?" + urllib.parse.urlencode(more)
            else:
                body["@odata.deltaLink"] = f"{GRAPH}/me/calendarView/delta?" + urllib.parse.urlencode({"$deltatoken": state["version"]})
        self.reply(200, body)

    def do_PATCH(self):
        url = urllib.parse.urlparse(self.path)
        sub = state["subscription"]
        if url.path.startswith("/v1.0/subscriptions/") and self.authorized():
            if not sub or url.path.rsplit("/", 1)[1] != sub["id"]:
                return self.reply(404, {"error": {"code": "ResourceNotFound", "message": "gone"}})
            sub["expirationDateTime"] = json.loads(self.body())["expirationDateTime"]
            log("renew", id=sub["id"])
            return self.reply(200, sub)
        self.reply(404, {"error": {"code": "NotFound", "message": url.path}})

    def do_POST(self):
        url = urllib.parse.urlparse(self.path)
        query = dict(urllib.parse.parse_qsl(url.query))
        raw = self.body()
        if url.path == "/organizations/oauth2/v2.0/token":
            return self.token(dict(urllib.parse.parse_qsl(raw.decode())))
        if url.path == "/v1.0/subscriptions" and self.authorized():
            return self.subscribe(json.loads(raw))
        if url.path.endswith("/reauthorize") and self.authorized():
            log("reauthorize")
            return self.reply(204)
        if url.path == "/test/events":
            return self.events(json.loads(raw))
        if url.path == "/test/notify":
            return self.notify("notificationUrl", {"changeType": "updated", "resource": "me/events/x"}, query.get("clientState"))
        if url.path == "/test/lifecycle":
            return self.notify("lifecycleNotificationUrl", {"lifecycleEvent": query["event"]}, query.get("clientState"))
        self.reply(404, {"error": {"code": "NotFound", "message": url.path}})

    def token(self, form):
        try:
            assert form["client_assertion_type"] == "urn:ietf:params:oauth:client-assertion-type:jwt-bearer", form
            check_assertion(form["client_assertion"], form["client_id"])
        except Exception as e:
            log("token-refused", why=repr(e))
            return self.reply(401, {"error": "invalid_client", "error_description": repr(e)})
        with lock:
            grant = form["grant_type"]
            if grant == "authorization_code":
                challenge, redirect, _ = state["codes"].pop(form["code"], (None, None, None))
                verified = challenge == b64url(hashlib.sha256(form["code_verifier"].encode()).digest())
                if not verified or redirect != form["redirect_uri"]:
                    log("token-refused", why="code")
                    return self.reply(400, {"error": "invalid_grant", "error_description": "code"})
            elif grant != "refresh_token" or form["refresh_token"] != state["refresh"]:
                log("token-refused", why="refresh")
                return self.reply(400, {"error": "invalid_grant", "error_description": "refresh"})
            state["grants"] = state.get("grants", 0) + 1
            n = state["grants"]
            state["access"], state["refresh"] = f"access-{n}", f"refresh-{n}"
            log("token", grant=grant, scope=form.get("scope"))
            return self.reply(200, {"access_token": state["access"], "refresh_token": state["refresh"], "expires_in": 3600, "token_type": "Bearer"})

    def subscribe(self, body):
        for key in ("notificationUrl", "lifecycleNotificationUrl"):
            token = f"validate-{state['subscriptions']}-{key}"
            status, text = post(body[key], query={"validationToken": token})
            log("validate", url=key, status=status, echoed=text == token)
            if status != 200 or text != token:
                return self.reply(400, {"error": {"code": "ValidationError", "message": key}})
        with lock:
            state["subscriptions"] += 1
            state["subscription"] = dict(body, id=f"sub-{state['subscriptions']}")
            log("subscribe", id=state["subscription"]["id"], resource=body["resource"], changeType=body["changeType"])
            return self.reply(201, state["subscription"])

    def events(self, spec):
        with lock:
            state["version"] += 1
            for event in spec.get("put", []):
                state["events"][event["id"]] = event
                state["versions"][event["id"]] = state["version"]
                state["removed"].pop(event["id"], None)
            for gone in spec.get("remove", []):
                state["events"].pop(gone, None)
                state["removed"][gone] = state["version"]
            if spec.get("forget"):
                state["subscription"] = None
        self.reply(200, {"version": state["version"]})

    def notify(self, url_key, item, client_state):
        sub = state["subscription"]
        notice = dict(item, subscriptionId=sub["id"], clientState=client_state or sub["clientState"])
        status, _ = post(sub[url_key], {"value": [notice]})
        log("sent", url=url_key, status=status, item=item)
        self.reply(200, {"status": status})

    def log_message(self, *args):
        pass


ThreadingHTTPServer(("127.0.0.1", 9440), Handler).serve_forever()
