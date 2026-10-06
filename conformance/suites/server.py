# SPDX-License-Identifier: AGPL-3.0-or-later
"""Discovery and error conventions: GET /server, 404 vs 501, the features contract."""

NAME = "server"

REQUIRED_FEATURE_KEYS = [
    "registration_open", "auth", "uploads", "music_identify", "podcast_discovery",
    "file_sync", "library_folders", "subsonic", "mail", "billing", "payments",
    "narration", "translation",
]

# A hosted-only path per feature key, to prove the 501 convention.
HOSTED_PATHS = {
    "billing": "/api/v1/family/billing",
    "narration": "/api/v1/audiobook-gen/languages",
    "translation": "/api/v1/podcast-translate/recent",
}


def run(ctx):
    if not ctx.discovery:
        ctx.skip("GET /server", "server predates discovery (404); baseline features assumed")
    else:
        info = ctx.server
        ctx.check("GET /server names the product", info.get("name") == "own.audio", str(info.get("name")))
        ctx.check("edition is foss or hosted", info.get("edition") in ("foss", "hosted"), str(info.get("edition")))
        ctx.check("version is a non-empty string", isinstance(info.get("version"), str) and bool(info["version"]))
        api = info.get("api") or {}
        ctx.check("api.version is 1", api.get("version") == 1, str(api.get("version")))
        ctx.check("api.revision is a positive integer", isinstance(api.get("revision"), int) and api["revision"] >= 1,
                  str(api.get("revision")))
        feats = info.get("features") or {}
        missing = [k for k in REQUIRED_FEATURE_KEYS if k not in feats]
        ctx.check("every documented features key is present", not missing, ", ".join(missing))
        bool_keys = [k for k in REQUIRED_FEATURE_KEYS if k not in ("auth", "uploads")]
        wrong = [k for k in bool_keys if not isinstance(feats.get(k), bool)]
        ctx.check("feature flags are booleans", not wrong, ", ".join(wrong))
        auth = feats.get("auth") or {}
        ctx.check("features.auth lists local, google, apple, microsoft",
                  all(isinstance(auth.get(p), bool) for p in ("local", "google", "apple", "microsoft")), str(auth))
        ctx.check("deprecations is a list", isinstance(info.get("deprecations"), list))
        ctx.check("features.auth agrees with /auth/providers",
                  all(auth.get(p) == bool((ctx.providers.get(p) or {}).get("enabled")) for p in ("google", "apple", "microsoft")))

    # Error conventions hold on every server, discovery or not.
    body = ctx.call("GET", "/api/v1/definitely-not-a-route", ctx.admin_token, expect=(404,), raw=True)
    ctx.check("unknown route is 404 not_found", b'"not_found"' in body, body[:80].decode(errors="replace"))

    for feature, path in HOSTED_PATHS.items():
        status = ctx.status_of("GET", path, ctx.admin_token)
        if ctx.feature(feature):
            ctx.check(f"{feature}: offered, so {path} is not 501", status != 501, str(status))
        elif ctx.discovery:
            got = ctx.call("GET", path, ctx.admin_token, expect=(501,), raw=True)
            ctx.check(f"{feature}: not offered, so {path} is 501 feature_unavailable",
                      b'"feature_unavailable"' in got and feature.encode() in got, got[:120].decode(errors="replace"))
        else:
            ctx.skip(f"{feature}: {path}", "pre-discovery server")
