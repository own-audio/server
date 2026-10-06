// SPDX-License-Identifier: AGPL-3.0-or-later
//! audio2 — private audiothek core library.
//!
//! This crate exposes the core application modules so that downstream
//! binaries (e.g. the SaaS edition) can compose their own `main()` and
//! extend the router with additional routes.
//!
//! ## Quick start (binary)
//!
//! The open-source binary calls [`app::run()`] with [`hooks::noop_factory()`]
//! and only core routes. The hosted edition builds its own binary: it calls
//! [`app::bootstrap()`] with its own [`hooks::Hooks`] implementation, merges
//! its routes into [`http::router::api_routes`] and finishes with
//! [`http::router::finalize()`]. See `hooks` for the seam.

pub mod app;
pub mod auth;
pub mod audiobooks;
pub mod dashboard;
pub mod db;
pub mod devices;
pub mod families;
pub mod filesync;
pub mod hooks;
pub mod http;
pub mod jobs;
pub mod library;
pub mod mail;
pub mod metadata;
pub mod music;
pub mod observability;
pub mod playback;
pub mod podcasts;
pub mod setup;
pub mod stats;
pub mod storage;
pub mod subsonic;
pub mod trash;
pub mod uploads;
pub mod users;
pub mod youtube;
