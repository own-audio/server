// SPDX-License-Identifier: AGPL-3.0-or-later
//! Observability module — logging init, metrics, health checks, tracing hooks.

/// Configure the global tracing subscriber.
/// Called from `app::run()` before any other subsystems start.
pub fn init(log_level: &str) {
    // Already initialised by main via tracing_subscriber::Registry.
    // This function is reserved for future metric / OTLP configuration.
    tracing::debug!(log_level, "observability initialised");
}
