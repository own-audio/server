// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::auth::LoginResponse;
use crate::db;
use axum::extract::{Json, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};
use argon2::Argon2;

// ── Router ────────────────────────────────────────────────────────────────

pub fn router(limits: &crate::http::rate_limit::Limiters) -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(setup_status))
        .routes(crate::http::openapi::map(routes!(complete_setup), |m| limits.setup.apply(m)))
}

// ── Types ─────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct SetupStatusResponse {
    /// `true` once an admin user has been created.
    pub setup_complete: bool,
    /// System health checks.
    pub checks: SystemChecks,
}

#[derive(Serialize, ToSchema)]
pub struct SystemChecks {
    /// The configured database is connected and migrations have run.
    pub database: bool,
    /// Storage backend is accessible.
    pub storage: bool,
    /// Which storage backend is configured.
    pub storage_backend: String,
}

#[derive(Deserialize, ToSchema)]
pub struct CompleteSetupRequest {
    pub email: String,
    pub password: String,
    pub display_name: String,
}

// ── Handlers ──────────────────────────────────────────────────────────────

/// GET /api/v1/setup/status — public, no auth required.
///
/// Returns whether initial setup is complete (at least one user exists)
/// and system health checks.
#[utoipa::path(get, path = "/status", tag = "setup",
    responses((status = 200, body = SetupStatusResponse), (status = 500, body = crate::http::openapi::ErrorBody)))]
async fn setup_status(
    State(state): State<AppState>,
) -> Result<Json<SetupStatusResponse>, AuthError> {
    let pool = state.db();

    // Check if any users exist
    let users = db::users::list_all(pool)
        .await
        .map_err(AuthError::Internal)?;

    let setup_complete = !users.is_empty();

    // Database check — if we got here, the DB is connected (the query above worked)
    let database_ok = true;

    // Storage check
    let storage_ok = check_storage(&state).await;

    let storage_backend = "local".to_string();

    Ok(Json(SetupStatusResponse {
        setup_complete,
        checks: SystemChecks {
            database: database_ok,
            storage: storage_ok,
            storage_backend,
        },
    }))
}

/// POST /api/v1/setup/complete — public, no auth required.
///
/// Creates the first admin user. Only works when no users exist.
/// Returns a JWT token so the user is auto-logged-in.
#[utoipa::path(post, path = "/complete", tag = "setup",
    request_body = CompleteSetupRequest,
    responses(
        (status = 201, description = "Admin created and signed in", body = crate::auth::LoginResponse),
        (status = 400, description = "Setup already complete, or invalid email, password (under 12 characters) or display name", body = crate::http::openapi::ErrorBody),
        (status = 429, description = "Rate limited; see `Retry-After`", body = crate::http::openapi::ErrorBody)))]
async fn complete_setup(
    State(state): State<AppState>,
    Json(body): Json<CompleteSetupRequest>,
) -> Result<(StatusCode, Json<LoginResponse>), AuthError> {
    let pool = state.db();

    // 1. Guard: only works when no users exist
    let users = db::users::list_all(pool)
        .await
        .map_err(AuthError::Internal)?;

    if !users.is_empty() {
        return Err(AuthError::BadRequest(
            "setup is already complete — an admin user already exists".into(),
        ));
    }

    // 2. Validate input
    let email = body.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return Err(AuthError::BadRequest("invalid email address".into()));
    }
    crate::auth::password::check(&body.password)?;
    let display_name = body.display_name.trim().to_string();
    if display_name.is_empty() {
        return Err(AuthError::BadRequest("display name is required".into()));
    }

    // 3. Hash password
    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(body.password.as_bytes(), &salt)
        .map_err(|e| AuthError::Internal(anyhow::anyhow!("password hash failed: {e}")))?
        .to_string();

    // 4. Create admin user
    let user = db::users::insert(pool, &email, &display_name, "admin")
        .await
        .map_err(AuthError::Internal)?;

    db::users::insert_local_identity(pool, user.id, &password_hash)
        .await
        .map_err(AuthError::Internal)?;

    let membership = db::families::home_new_user(pool, user.id, state.hooks().one_family())
        .await
        .map_err(AuthError::Internal)?;
    state
        .hooks()
        .user_created(pool, membership.family_id, user.id)
        .await
        .map_err(AuthError::Internal)?;
    crate::auth::verification::born(&state, &user, membership.family_id, true).await?;

    tracing::info!(
        email = %user.email,
        "initial admin user created via setup wizard"
    );

    // 5. Auto-login: mint access + refresh tokens
    let issued = crate::auth::issue_tokens(&state, &user, None, Some("web"), None).await?;

    Ok((
        StatusCode::CREATED,
        Json(crate::auth::login_response(issued, user)),
    ))
}

/// Check that the storage backend is functional.
async fn check_storage(state: &AppState) -> bool {
    let test_key = "__setup_check_test__";
    let test_data = bytes::Bytes::from_static(b"audio2-setup-check");

    // Try to write, read, and delete a small test file
    match state.storage().put(test_key, test_data.clone(), "text/plain").await {
        Ok(_) => {
            // Try to read it back
            let read_ok = state.storage().get(test_key).await.is_ok();
            // Clean up
            let _ = state.storage().delete(test_key).await;
            read_ok
        }
        Err(e) => {
            tracing::warn!("storage check failed: {e}");
            false
        }
    }
}
