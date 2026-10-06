// SPDX-License-Identifier: AGPL-3.0-or-later
//! The open-source own.audio server binary: the core with no edition hooks.
//! The hosted edition is a separate crate (`hosted/`) that composes the same
//! library with billing, payments and the pipelines.

use anyhow::Context;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env before anything else so RUST_LOG etc. are set
    let _ = dotenvy::dotenv();

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "audio2=debug,info".parse().unwrap()))
        .init();

    audio2::app::run(audio2::hooks::noop_factory())
        .await
        .context("application error")
}
