// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/me`: who is signed in, in which household, allowed to do what; the
//! account's signed-in devices and its personal access tokens.

use crate::app::AppState;
use crate::auth::audit::{self, RequestMeta};
use crate::auth::middleware::AuthUser;
use crate::db;
use crate::families::FamilyContext;
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Serialize, ToSchema)]
pub struct Me {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub email_verified: bool,
    /// `admin` runs the server; `user` is everyone else.
    pub instance_role: String,
    pub family: MeFamily,
}

#[derive(Serialize, ToSchema)]
pub struct MeFamily {
    pub id: String,
    /// `admin` or `member`.
    pub role: String,
    /// What the caller may do in it, family admins' rights included.
    pub can_upload: bool,
    pub can_generate: bool,
}

/// The signed-in account and its household.
#[utoipa::path(get, path = "/me", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user"))),
    responses((status = 200, body = Me)))]
pub async fn me(family: FamilyContext, State(state): State<AppState>) -> Result<Json<Me>, Problem> {
    let user = db::users::find_by_id(state.db(), family.user_id)
        .await?
        .ok_or_else(|| Problem::new(StatusCode::UNAUTHORIZED, "unauthorized"))?;
    Ok(Json(Me {
        id: user.id.to_string(),
        email: user.email,
        display_name: user.display_name,
        email_verified: user.email_verified_at.is_some(),
        instance_role: if user.role == "admin" { "admin" } else { "user" }.to_string(),
        family: MeFamily {
            id: family.family_id.to_string(),
            role: if family.family_role == "family_admin" { "admin" } else { "member" }.to_string(),
            can_upload: family.effective_can_upload(),
            can_generate: family.effective_can_generate(),
        },
    }))
}

// ── Sessions ────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct Session {
    /// The device's id; signing it out ends its refresh token and its access tokens.
    pub id: String,
    pub device_name: Option<String>,
    pub device_kind: String,
    pub signed_in_at: String,
    pub last_used_at: Option<String>,
    pub expires_at: String,
    /// The device making this request.
    pub current: bool,
}

/// The devices signed in to this account.
#[utoipa::path(get, path = "/me/sessions", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    responses((status = 200, body = Vec<Session>)))]
pub async fn sessions(auth: AuthUser, State(state): State<AppState>) -> Result<Json<Vec<Session>>, Problem> {
    let devices = db::refresh_tokens::list_devices(state.db(), auth.user_id).await?;
    Ok(Json(
        devices
            .into_iter()
            .map(|d| Session {
                current: auth.chain_id == Some(d.chain_id),
                id: d.chain_id.to_string(),
                device_name: d.device_name,
                device_kind: d.device_kind,
                signed_in_at: d.signed_in_at.to_rfc3339(),
                last_used_at: d.last_used_at.map(|t| t.to_rfc3339()),
                expires_at: d.expires_at.to_rfc3339(),
            })
            .collect(),
    ))
}

/// Sign one device out, at once.
#[utoipa::path(delete, path = "/me/sessions/{id}", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    params(("id" = Uuid, Path, description = "From `GET /me/sessions`")),
    responses((status = 204, description = "Signed out"), (status = 404, description = "Not a device of this account", body = crate::v2::problem::ProblemBody)))]
pub async fn delete_session(
    auth: AuthUser,
    State(state): State<AppState>,
    meta: RequestMeta,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    let pool = state.db();
    if !db::refresh_tokens::list_devices(pool, auth.user_id).await?.iter().any(|d| d.chain_id == id) {
        return Err(Problem::not_found());
    }
    db::refresh_tokens::revoke_chain(pool, id, auth.user_id).await?;
    db::sessions::revoke_chain(pool, auth.user_id, id).await?;
    audit::record(&state, &meta, Some(auth.user_id), None, "session.revoked", serde_json::json!({ "chain_id": id })).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Sign out everywhere, this device included.
#[utoipa::path(delete, path = "/me/sessions", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    responses((status = 204, description = "Every device signed out")))]
pub async fn delete_sessions(auth: AuthUser, State(state): State<AppState>, meta: RequestMeta) -> Result<StatusCode, Problem> {
    let pool = state.db();
    db::refresh_tokens::revoke_all_for_user(pool, auth.user_id, None).await?;
    db::sessions::revoke_all_for_user(pool, auth.user_id).await?;
    audit::record(&state, &meta, Some(auth.user_id), None, "session.revoked_all", serde_json::json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

// ── Personal access tokens ──────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct PersonalToken {
    pub id: String,
    pub name: String,
    pub scopes: Vec<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub expires_at: Option<String>,
    /// The secret: only in the answer that created it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

impl From<db::personal_tokens::PersonalToken> for PersonalToken {
    fn from(t: db::personal_tokens::PersonalToken) -> Self {
        PersonalToken {
            id: t.id.to_string(),
            name: t.name,
            scopes: t.scopes,
            created_at: t.created_at.to_rfc3339(),
            last_used_at: t.last_used_at.map(|d| d.to_rfc3339()),
            expires_at: t.expires_at.map(|d| d.to_rfc3339()),
            token: None,
        }
    }
}

#[derive(Deserialize, ToSchema)]
pub struct CreateTokenRequest {
    /// What it is for: "backup script".
    pub name: String,
    /// From `library:read`, `library:write`, `playback`, `user:write`, `remote`,
    /// `family:admin`, `admin` — only those the account has.
    pub scopes: Vec<String>,
    /// Days until it stops working; absent for never.
    pub expires_in_days: Option<i64>,
}

/// The account's personal access tokens, without their secrets.
#[utoipa::path(get, path = "/me/tokens", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    responses((status = 200, body = Vec<PersonalToken>)))]
pub async fn tokens(auth: AuthUser, State(state): State<AppState>) -> Result<Json<Vec<PersonalToken>>, Problem> {
    Ok(Json(db::personal_tokens::list(state.db(), auth.user_id).await?.into_iter().map(Into::into).collect()))
}

/// A token for a script: scoped, named, shown once. Only a signed-in session
/// can make one, so a token cannot mint a stronger token.
#[utoipa::path(post, path = "/me/tokens", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    request_body = CreateTokenRequest,
    responses(
        (status = 201, description = "The token, with its secret this once", body = PersonalToken),
        (status = 400, description = "`invalid_scope`, or a missing name", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Asked from a personal token", body = crate::v2::problem::ProblemBody)))]
pub async fn create_token(
    family: FamilyContext,
    auth: AuthUser,
    State(state): State<AppState>,
    meta: RequestMeta,
    Json(body): Json<CreateTokenRequest>,
) -> Result<(StatusCode, Json<PersonalToken>), Problem> {
    if auth.scopes.is_some() {
        return Err(Problem::forbidden().detail("sign in to make a token"));
    }
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(Problem::bad_request("invalid_body", "a name of 1 to 100 characters is required")
            .field("/name", "invalid", "1 to 100 characters"));
    }
    let allowed = |s: &str| match s {
        "admin" => auth.role == "admin",
        "family:admin" => family.is_family_admin(),
        other => crate::v2::auth::SCOPES.contains(&other),
    };
    let mut scopes: Vec<String> = Vec::new();
    for scope in &body.scopes {
        if !allowed(scope) {
            return Err(Problem::bad_request("invalid_scope", format!("`{scope}` is not a scope this account can give"))
                .field("/scopes", "invalid_scope", scope.clone()));
        }
        if !scopes.contains(scope) {
            scopes.push(scope.clone());
        }
    }
    if scopes.is_empty() {
        return Err(Problem::bad_request("invalid_scope", "at least one scope is required"));
    }
    let expires_at = match body.expires_in_days {
        None => None,
        Some(days) if (1..=3650).contains(&days) => Some(chrono::Utc::now() + chrono::Duration::days(days)),
        Some(_) => return Err(Problem::bad_request("invalid_body", "expires_in_days is 1 to 3650").field("/expires_in_days", "out_of_range", "1 to 3650")),
    };
    let mut secret = [0u8; 32];
    argon2::password_hash::rand_core::RngCore::fill_bytes(&mut argon2::password_hash::rand_core::OsRng, &mut secret);
    let token = format!("{}{}", db::personal_tokens::PREFIX, base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, secret));
    let created = db::personal_tokens::create(state.db(), auth.user_id, name, &db::refresh_tokens::hash_token(&token), &scopes, expires_at).await?;
    audit::record(&state, &meta, Some(auth.user_id), None, "token.created", serde_json::json!({ "token_id": created.id, "name": name, "scopes": scopes })).await;
    let mut answer = PersonalToken::from(created);
    answer.token = Some(token);
    Ok((StatusCode::CREATED, Json(answer)))
}

/// Revoke a personal access token.
#[utoipa::path(delete, path = "/me/tokens/{id}", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    params(("id" = Uuid, Path, description = "From `GET /me/tokens`")),
    responses((status = 204, description = "Revoked"), (status = 404, description = "Not a live token of this account", body = crate::v2::problem::ProblemBody)))]
pub async fn delete_token(auth: AuthUser, State(state): State<AppState>, meta: RequestMeta, Path(id): Path<Uuid>) -> Result<StatusCode, Problem> {
    if !db::personal_tokens::revoke(state.db(), auth.user_id, id).await? {
        return Err(Problem::not_found());
    }
    audit::record(&state, &meta, Some(auth.user_id), None, "token.revoked", serde_json::json!({ "token_id": id })).await;
    Ok(StatusCode::NO_CONTENT)
}
