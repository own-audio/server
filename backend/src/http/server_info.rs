// SPDX-License-Identifier: AGPL-3.0-or-later
//! `GET /api/v1/server` — what this server is and what it offers, so a
//! client asks instead of assuming. The contract is
//! `own-audio-foss/docs/API_COMPATIBILITY.md` §3: every optional capability
//! has a key under `features`, `true` only when fully configured; unknown
//! keys are ignored by clients and missing keys read as `false`.

use crate::app::AppState;
use axum::Json;
use axum::extract::State;
use serde_json::{Map, Value, json};

/// The path major, `/api/v1`.
pub const API_VERSION: u32 = 1;

/// Bumped by every additive change to the v1 contract (new endpoint, field,
/// enum value or `features` key) — the one number a client may compare.
/// 1: this endpoint, `GET /family/storage`, the `feature_unavailable` 501.
/// 2: `demo` in this endpoint's response.
/// 3: `source` and `read_only` on tracks and books; `GET /library/folders`,
///    `POST /library/folders/scan`; stream URLs may be server media links.
pub const API_REVISION: u32 = 4;

pub async fn server_info(State(state): State<AppState>) -> Json<Value> {
    let cfg = state.config();
    let providers = crate::auth::providers_response(&cfg.auth);

    let mut features = Map::new();
    features.insert("registration_open".into(), json!(cfg.auth.registration_open));
    features.insert(
        "auth".into(),
        json!({
            "local": providers.local,
            "google": providers.google.enabled,
            "apple": providers.apple.enabled,
            "microsoft": providers.microsoft.enabled,
        }),
    );
    features.insert("uploads".into(), json!({ "presigned": true, "multipart_max_bytes": Value::Null }));
    // Identify and discovery need a metadata provider; today that is the
    // configured mirror service, in Phase 4 of the FOSS plan also the public
    // MusicBrainz API.
    features.insert("music_identify".into(), json!(cfg.metadata.is_some()));
    features.insert("podcast_discovery".into(), json!(cfg.metadata.is_some()));
    features.insert("file_sync".into(), json!(true));
    features.insert(
        "library_folders".into(),
        json!(crate::library_folders::configured(cfg).is_ok_and(|f| !f.is_empty())),
    );
    features.insert("subsonic".into(), json!(true));
    let mail = cfg.mail.as_ref().is_some_and(|m| m.smtp_host().is_some() || m.jmap_base_url().is_some());
    features.insert("mail".into(), json!(mail));
    // Revision 4: every account joins the one family; clients hide "leave".
    features.insert("one_family".into(), json!(state.hooks().one_family()));
    // The edition's own keys; absent in the open-source edition, so set the
    // documented defaults first and let the hooks overwrite.
    for key in ["billing", "payments", "narration", "translation"] {
        features.insert(key.into(), json!(false));
    }
    for (k, v) in state.hooks().features() {
        features.insert(k, v);
    }

    // Only a public demo sets this; everywhere else the key is absent.
    let demo = cfg.server.demo.as_ref().map(|d| json!({ "email": d.email, "password": d.password }));

    let mut body = json!({
        "name": "own.audio",
        "edition": state.hooks().edition(),
        "version": env!("CARGO_PKG_VERSION"),
        "api": { "version": API_VERSION, "revision": API_REVISION },
        "features": Value::Object(features),
        "deprecations": [],
    });
    if let Some(demo) = demo {
        body["demo"] = demo;
    }
    Json(body)
}
