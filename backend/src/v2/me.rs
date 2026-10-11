// SPDX-License-Identifier: AGPL-3.0-or-later
//! `GET /me`: who is signed in, in which household, allowed to do what.

use crate::app::AppState;
use crate::db;
use crate::families::FamilyContext;
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;

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
        .ok_or_else(|| Problem::new(axum::http::StatusCode::UNAUTHORIZED, "unauthorized"))?;
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
