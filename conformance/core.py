# SPDX-License-Identifier: AGPL-3.0-or-later
"""Shared runtime for the conformance suites.

A suite is a module in `suites/` with a `NAME`, an optional `REQUIRES` set
(`"db"` for suites that need SQL access to the server's database, `"local"`
for suites that need the server to reach this machine) and a `run(ctx)`
function. Everything a suite touches on the wire goes through `Ctx`, so the
suite itself never knows whether it is talking to a laptop, the compose
stack, canary or production.
"""
from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from dataclasses import dataclass, field
from typing import Any, Callable, Iterable

# Cloudflare fronts the hosted API and answers urllib's default User-Agent
# with a 403 "error code: 1010" before the request reaches the backend.
USER_AGENT = "own-audio-conformance/0.1"

# What a server older than contract revision 1 is assumed to offer. Used when
# GET /api/v1/server answers 404 (see docs/API_COMPATIBILITY.md §3).
BASELINE_FEATURES: dict[str, Any] = {
    "registration_open": False,
    "auth": {"local": True, "google": False, "apple": False, "microsoft": False},
    "uploads": {"presigned": True, "multipart_max_bytes": None},
    "narration": False,
    "translation": False,
    "music_identify": False,
    "podcast_discovery": False,
    "file_sync": True,
    "library_folders": False,
    "subsonic": True,
    "mail": False,
    "billing": False,
    "payments": False,
}


class ApiError(AssertionError):
    def __init__(self, method: str, path: str, status: int, body: bytes, expect: Iterable[int]) -> None:
        self.method, self.path, self.status, self.body = method, path, status, body
        super().__init__(f"{method} {path} -> {status} (expected {tuple(expect)}): {body[:300]!r}")


class Skip(Exception):
    """Raised by a suite (or by Ctx) to skip the rest of the suite cleanly."""


@dataclass
class Result:
    suite: str
    label: str
    status: str  # "ok" | "FAIL" | "skip"
    detail: str = ""


@dataclass
class Ctx:
    base_url: str
    admin_email: str
    admin_password: str
    compose_dir: pathlib.Path | None = None
    database: str = "audio2"
    require_media: bool = False
    timeout: float = 60.0
    verbose: bool = True

    suite: str = ""
    results: list[Result] = field(default_factory=list)
    admin_token: str = ""
    features: dict[str, Any] = field(default_factory=dict)
    server: dict[str, Any] = field(default_factory=dict)
    discovery: bool = False
    providers: dict[str, Any] = field(default_factory=dict)
    sfx: str = field(default_factory=lambda: uuid.uuid4().hex[:8])
    _cleanups: list[tuple[str, Callable[[], None]]] = field(default_factory=list)

    # ── wire ───────────────────────────────────────────────────────────────
    def url(self, path: str) -> str:
        if path.startswith(("http://", "https://")):
            return path
        if not path.startswith("/"):
            path = "/" + path
        return self.base_url + path

    def call(
        self,
        method: str,
        path: str,
        token: str | None = None,
        body: Any = None,
        expect: Iterable[int] = (200, 201, 204),
        multipart: dict[str, Any] | None = None,
        headers: dict[str, str] | None = None,
        raw: bool = False,
    ) -> Any:
        """One HTTP call. Returns parsed JSON (None for empty/204), or bytes with raw=True.

        `multipart` is {field: str | (filename, bytes)}; file parts are sent as audio/mpeg.
        A status outside `expect` raises ApiError (an AssertionError, so a suite's
        try/finally cleanup still runs and the runner records a FAIL).
        """
        hdrs = {"Accept": "application/json", "User-Agent": USER_AGENT}
        if headers:
            hdrs.update(headers)
        if token:
            hdrs["Authorization"] = f"Bearer {token}"
        data = None
        if multipart is not None:
            boundary = "----ownaudio" + uuid.uuid4().hex
            parts: list[bytes] = []
            for name, value in multipart.items():
                parts.append(f"--{boundary}\r\n".encode())
                if isinstance(value, tuple):
                    fname, content = value
                    parts.append(
                        f'Content-Disposition: form-data; name="{name}"; filename="{fname}"\r\n'
                        f"Content-Type: audio/mpeg\r\n\r\n".encode()
                    )
                    parts.append(content)
                else:
                    parts.append(f'Content-Disposition: form-data; name="{name}"\r\n\r\n'.encode())
                    parts.append(str(value).encode())
                parts.append(b"\r\n")
            parts.append(f"--{boundary}--\r\n".encode())
            data = b"".join(parts)
            hdrs["Content-Type"] = f"multipart/form-data; boundary={boundary}"
        elif body is not None:
            data = json.dumps(body).encode()
            hdrs["Content-Type"] = "application/json"
        req = urllib.request.Request(self.url(path), data=data, headers=hdrs, method=method.upper())
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as r:
                status, out = r.status, r.read()
        except urllib.error.HTTPError as e:
            status, out = e.code, e.read()
        if status not in tuple(expect):
            raise ApiError(method, path, status, out, expect)
        if raw:
            return out
        return json.loads(out) if out and status != 204 else None

    def status_of(self, method: str, path: str, token: str | None = None, body: Any = None) -> int:
        """The status code of a call, whatever it is."""
        try:
            self.call(method, path, token, body, expect=())
        except ApiError as e:
            return e.status
        raise RuntimeError("unreachable")

    def probe_media(self, url_or_path: str, expect: Iterable[int] = (200, 206)) -> int:
        """A ranged GET, not HEAD: presigned S3 URLs are signed for GET only."""
        req = urllib.request.Request(
            self.url(url_or_path), method="GET", headers={"Range": "bytes=0-1023", "User-Agent": USER_AGENT}
        )
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as r:
                r.read()
                status = r.status
        except urllib.error.HTTPError as e:
            status = e.code
        if status not in tuple(expect):
            raise ApiError("GET", url_or_path, status, b"", expect)
        return status

    def put_bytes(self, url: str, content: bytes, content_type: str, expect: Iterable[int] = (200, 201, 204)) -> int:
        """PUT a body to a (presigned or server) URL; a status outside `expect` raises ApiError."""
        req = urllib.request.Request(
            url, data=content, method="PUT", headers={"Content-Type": content_type, "User-Agent": USER_AGENT}
        )
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as r:
                status = r.status
        except urllib.error.HTTPError as e:
            status = e.code
        if status not in tuple(expect):
            raise ApiError("PUT", url, status, b"", expect)
        return status

    # ── discovery ──────────────────────────────────────────────────────────
    def discover(self) -> None:
        health = self.call("GET", "/health")
        if health.get("status") != "ok":
            raise AssertionError(f"/health answered {health!r}")
        try:
            self.server = self.call("GET", "/api/v1/server")
            self.discovery = True
            self.features = dict(BASELINE_FEATURES)
            self.features.update(self.server.get("features") or {})
        except ApiError as e:
            if e.status != 404:
                raise
            self.server, self.discovery, self.features = {}, False, dict(BASELINE_FEATURES)
        try:
            self.providers = self.call("GET", "/api/v1/auth/providers")
            auth = dict(self.features.get("auth") or {})
            for p in ("google", "apple", "microsoft"):
                info = self.providers.get(p)
                auth[p] = bool(info and info.get("enabled"))
            auth["local"] = bool(self.providers.get("local", True))
            self.features["auth"] = auth
        except ApiError:
            pass

    def feature(self, key: str, default: bool = False) -> bool:
        cur: Any = self.features
        for part in key.split("."):
            if not isinstance(cur, dict) or part not in cur:
                return default
            cur = cur[part]
        return bool(cur)

    def login_admin(self) -> None:
        r = self.call("POST", "/api/v1/auth/login", body={"email": self.admin_email, "password": self.admin_password})
        self.admin_token = r["token"]

    # ── results ────────────────────────────────────────────────────────────
    def log(self, msg: str) -> None:
        if self.verbose:
            print(msg, flush=True)

    def check(self, label: str, cond: bool, detail: str = "") -> bool:
        self.results.append(Result(self.suite, label, "ok" if cond else "FAIL", detail))
        self.log(f"  {'ok  ' if cond else 'FAIL'} {label}{(' — ' + detail) if (detail and not cond) else ''}")
        return bool(cond)

    def ok(self, label: str, detail: str = "") -> None:
        self.check(label, True, detail)

    def fail(self, label: str, detail: str = "") -> None:
        self.check(label, False, detail)

    def skip(self, label: str, reason: str) -> None:
        self.results.append(Result(self.suite, label, "skip", reason))
        self.log(f"  skip {label} — {reason}")

    def require_feature(self, key: str) -> None:
        if not self.feature(key):
            raise Skip(f"server reports features.{key} = false")

    # ── fixtures ───────────────────────────────────────────────────────────
    @staticmethod
    def audio(size_blocks: int = 256) -> bytes:
        """Bytes that pass as an MP3 upload: an ID3 header and random padding."""
        return b"ID3" + uuid.uuid4().bytes * size_blocks

    def make_user(
        self, label: str, password: str, prefix: str | None = None, role: str = "user",
        display_name: str | None = None,
    ) -> tuple[str, str, str]:
        """Create a user as the admin and log them in. Returns (id, email, token). Registered for cleanup."""
        prefix = prefix or self.suite
        email = f"{prefix}-{label}-{self.sfx}@example.com"
        u = self.call(
            "POST", "/api/v1/auth/admin-create-user", self.admin_token,
            {"email": email, "password": password, "display_name": display_name or f"{label} {self.sfx}", "role": role},
            expect=(200, 201),
        )
        tok = self.call("POST", "/api/v1/auth/login", body={"email": email, "password": password})["token"]
        self.track_user(u["id"], email)
        return u["id"], email, tok

    def track_user(self, uid: str, label: str = "") -> None:
        """Register a user the suite created some other way (register, claim) for deletion at the end."""
        self.on_cleanup(f"user {label or uid}", lambda: self.delete_user(uid))

    def delete_user(self, uid: str) -> None:
        self.call("DELETE", f"/api/v1/users/{uid}", self.admin_token, expect=(200, 204, 401, 404))

    def upload_track(
        self, token: str, title: str, artist: str | None = "Test Artist", album: str | None = "Test Album",
        visibility: str = "private", genre: str | None = None, content: bytes | None = None,
    ) -> Any:
        """Multipart track upload. Pass artist=None / album=None to omit the field and test the server's defaults."""
        fields: dict[str, Any] = {"title": title, "visibility": visibility}
        if artist is not None:
            fields["artist"] = artist
        if album is not None:
            fields["album"] = album
        if genre:
            fields["genre"] = genre
        fields["file"] = (f"{uuid.uuid4().hex}.mp3", content or self.audio())
        return self.call("POST", "/api/v1/music/tracks/upload", token, multipart=fields)

    def presign_upload(self, token: str, kind: str, filename: str, content: bytes | None = None) -> str:
        """Presigned direct upload. Returns the object key to register."""
        content = content or self.audio()
        p = self.call(
            "POST", "/api/v1/uploads/presign", token,
            {"kind": kind, "filename": filename, "content_type": "audio/mpeg", "size_bytes": len(content)},
        )
        self.put_bytes(p["url"], content, p["content_type"])
        return p["object_key"]

    # ── database access (optional) ─────────────────────────────────────────
    @property
    def db_available(self) -> bool:
        return self.compose_dir is not None

    @property
    def is_local(self) -> bool:
        host = urllib.parse.urlparse(self.base_url).hostname or ""
        return host in ("127.0.0.1", "localhost", "::1")

    def sql(self, query: str) -> str:
        """Run SQL on the server's Postgres through `docker compose exec`. Only with --compose-dir."""
        if self.compose_dir is None:
            raise Skip("needs --compose-dir for SQL access to the server database")
        out = subprocess.run(
            ["docker", "compose", "exec", "-T", "postgres", "psql", "-U", "audio2", "-d", self.database, "-At", "-c", query],
            cwd=self.compose_dir, capture_output=True, text=True, check=True,
        )
        return out.stdout.strip()

    # ── cleanup ────────────────────────────────────────────────────────────
    def on_cleanup(self, what: str, fn: Callable[[], None]) -> None:
        self._cleanups.append((what, fn))

    def run_cleanups(self) -> None:
        while self._cleanups:
            what, fn = self._cleanups.pop()
            try:
                fn()
            except Exception as e:  # noqa: BLE001 — cleanup is best effort
                self.log(f"  warn cleanup {what}: {e}")

    def wait_until(self, pred: Callable[[], bool], timeout: float = 30.0, every: float = 0.5) -> bool:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if pred():
                return True
            time.sleep(every)
        return pred()
