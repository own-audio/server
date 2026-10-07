// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::Context;
use serde::Deserialize;

/// Top-level application configuration, loaded from env + optional config file.
#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    /// Bind address for the HTTP server.
    pub server: ServerConfig,

    /// PostgreSQL connection URL.
    pub database_url: String,

    /// `DATABASE_MAX_CONNECTIONS` — the server's connection pool size. Unset
    /// ⇒ [`crate::db::DEFAULT_MAX_CONNECTIONS`] (10), sized for one family; a
    /// busy multi-family deployment raises it.
    pub database_max_connections: Option<u32>,

    /// Storage configuration (S3-compatible object storage).
    pub storage: StorageConfig,

    /// Auth provider configuration.
    pub auth: AuthConfig,

    /// Google API keys the core itself uses — today only the Books API for
    /// audiobook identify. The hosted edition reads its Translation, Gemini
    /// and Text-to-Speech keys from the same `GOOGLE_CLOUD__*` section into
    /// its own `HostedConfig` (`billing::config`); they are not core config.
    pub google_cloud: Option<GoogleCloudConfig>,

    /// The self-hosted MusicBrainz mirror service (`music-metadata`) that music
    /// identification queries. Unset means the identify endpoints answer 503
    /// rather than silently falling back to the public MusicBrainz API — that
    /// fallback is rate-limited to ~1 req/s per outbound IP for the whole
    /// deployment, so reaching it by accident would look like an outage under
    /// any real load. See docs/music-metadata-plan.md.
    pub metadata: Option<MetadataConfig>,

    /// Outbound mail (JMAP) for family invite emails. Unset ⇒ invite
    /// creation still succeeds and returns the code/link for out-of-band
    /// delivery; only the email step no-ops. See `crate::mail`.
    pub mail: Option<MailConfig>,

    /// Minimum log level for the application.
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// Comma-separated allowlist of job types this process's worker loop
    /// claims (e.g. `"feed_refresh,gen_pipeline"`). Unset means "claim
    /// everything" (default, single-container deployments). Used to split
    /// CPU-heavy `assemble_audiobook` jobs (ffmpeg) onto a dedicated,
    /// CPU-limited container — see docker-compose.yml's `assembler` service.
    pub worker_job_types: Option<String>,
    /// `WORKER_MAX_CONCURRENT_JOBS` — how many claimed jobs this process runs
    /// at once. Unset ⇒ 4. The production `jobs` container sets 1 so a job
    /// always waits for the previous one; on a 4 GB host that is the point.
    pub worker_max_concurrent_jobs: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,

    #[serde(default = "default_port")]
    pub port: u16,

    /// Optional base URL used in links / OIDC redirect URIs. This is the API's
    /// own origin — what a client talks to, not what a person browses.
    pub base_url: Option<String>,

    /// Where the web console lives, for links a person is expected to click:
    /// family invites, device sign-in, Stripe returns, narration mail.
    ///
    /// Unset ⇒ [`Self::base_url`], which is correct whenever one origin serves
    /// both. The hosted deployment does not: the API is `api.own.audio` and the
    /// console is `app.own.audio`, so a join link built from `base_url` points
    /// at a host that serves no pages and 404s.
    pub app_base_url: Option<String>,

    /// Per-IP limits on the routes an attacker would hammer. See
    /// [`RateLimitConfig`]; `SERVER__RATE_LIMIT__ENABLED=false` turns them off.
    #[serde(default)]
    pub rate_limit: RateLimitConfig,

    /// A public demo's shared sign-in, shown on the console's sign-in screen
    /// so a visitor can try the server without asking for an account. Both
    /// `SERVER__DEMO__EMAIL` and `SERVER__DEMO__PASSWORD` are published
    /// through `GET /api/v1/server` to anyone, so set them only on a server
    /// whose whole point is to be tried by strangers.
    pub demo: Option<DemoConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DemoConfig {
    pub email: String,
    pub password: String,
}

/// Per-client-IP request limits for the unauthenticated, guessable routes:
/// login, token refresh, TV device codes, join codes, first-run setup. The
/// hosted service sits behind Cloudflare; a self-hosted install on a bare VPS
/// does not, and without this a password can be tried as fast as the network
/// allows. Limits are per minute, with the same number as the burst.
///
/// The client IP is the peer address, unless the peer is a private or
/// loopback address (a reverse proxy, cloudflared, Docker's bridge) *or*
/// `trust_proxy_headers` is set — then `CF-Connecting-IP`, the first
/// `X-Forwarded-For` entry or `X-Real-IP` is used. Trusting those headers
/// from a public peer would let a client pick its own key, so it is never
/// the default.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RateLimitConfig {
    pub enabled: bool,
    pub login_per_minute: u32,
    pub refresh_per_minute: u32,
    pub device_per_minute: u32,
    pub join_per_minute: u32,
    pub setup_per_minute: u32,
    pub trust_proxy_headers: bool,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            login_per_minute: 30,
            refresh_per_minute: 120,
            device_per_minute: 60,
            join_per_minute: 30,
            setup_per_minute: 10,
            trust_proxy_headers: false,
        }
    }
}

impl ServerConfig {
    /// The origin to build person-facing links from. Never the API origin unless
    /// that is genuinely also the console.
    pub fn web_base(&self) -> Option<&str> {
        self.app_base_url.as_deref().or(self.base_url.as_deref())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    /// S3 endpoint the backend talks to, e.g. `http://garage:3900`.
    pub endpoint: String,

    /// Endpoint reachable from browsers, used only for presigning URLs.
    /// Defaults to `endpoint` when unset. In Docker the backend reaches
    /// Garage via the service name while browsers need the host address,
    /// and SigV4 signatures cover the Host header, so both must be known.
    pub public_endpoint: Option<String>,

    #[serde(default = "default_s3_region")]
    pub region: String,

    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,

    /// Force path-style addressing (required by Garage and MinIO).
    #[serde(default = "default_true")]
    pub path_style: bool,

    /// Apply a permissive GET/HEAD CORS policy to the bucket at startup.
    #[serde(default = "default_true")]
    pub set_cors: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    /// Secret key used to sign session JWTs.
    pub session_secret: String,

    /// Session lifetime in seconds. Deprecated alias for `access_ttl_secs`;
    /// still honored as the access-token TTL when `access_ttl_secs` is unset.
    #[serde(default = "default_session_ttl")]
    pub session_ttl_secs: u64,

    /// Access-JWT lifetime in seconds. Unset ⇒ falls back to
    /// `session_ttl_secs` (7 days) so existing deployments keep long-lived
    /// tokens until their clients adopt the refresh flow; set to e.g. 3600
    /// once they do.
    pub access_ttl_secs: Option<u64>,

    /// Refresh-token lifetime in seconds (default 90 days).
    #[serde(default = "default_refresh_ttl")]
    pub refresh_ttl_secs: u64,

    /// Enable local username/password authentication.
    #[serde(default = "default_true")]
    pub local_enabled: bool,

    /// Allow public self-registration. When false, only admins can create accounts.
    #[serde(default)]
    pub registration_open: bool,

    /// Seed `admin@audio2.local` / `admin` when the users table is empty.
    /// **Off by default**: it is a fixed, documented credential with the admin
    /// role, so anywhere reachable must create its first admin through
    /// `POST /setup/complete` instead. The local compose stack turns it on.
    #[serde(default)]
    pub dev_seed_admin: bool,

    /// Microsoft Sign-In (Entra, `common` authority). Unset ⇒ `/auth/microsoft`
    /// answers 501 and `/auth/providers` reports it disabled.
    pub microsoft: Option<MicrosoftAuthConfig>,

    /// Google Sign-In. Unset ⇒ `/auth/google` answers 501 and
    /// `GET /auth/providers` reports it disabled. See docs/sso-payments-plan.md.
    pub google: Option<GoogleAuthConfig>,

    /// Sign in with Apple. Unset ⇒ `/auth/apple` answers 501 and
    /// `GET /auth/providers` reports it disabled. See docs/sso-payments-plan.md.
    pub apple: Option<AppleAuthConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GoogleCloudConfig {
    /// API key for the Books API, used by the audiobook identify flow
    /// (`metadata::google_books`).
    ///
    /// **Optional, but not really.** The Books API answers unauthenticated
    /// requests against a per-IP anonymous quota shared with every other
    /// keyless caller on that address — which is observably exhausted a lot of
    /// the time (a plain `curl` from a home connection came back
    /// `429 RESOURCE_EXHAUSTED` during development). A key moves the quota to
    /// our own project. Unset still works; it just fails whenever the shared
    /// pool is dry, and there is no way to tell that apart from an outage.
    pub books_api_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetadataConfig {
    /// Base URL of the `music-metadata` service, e.g. `http://music-metadata:8090`.
    /// A private-network address: the service holds no per-user data but has no
    /// business being reachable from the internet.
    pub base_url: String,

    /// The pre-shared key the service checks (`Authorization: Bearer …`). A
    /// second line of defence behind network isolation, not the primary one.
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MailConfig {
    /// JMAP server base URL, e.g. `https://mail.example.com`.
    pub jmap_base_url: String,
    pub jmap_user: String,
    pub jmap_password: String,
    /// Envelope + header From address. The hosted own.audio instance reuses
    /// `hello@own.audio` (the same address the waitlist already sends
    /// from) rather than minting a dedicated alias.
    pub from_address: String,
    /// Display name on the From header, e.g. `own.audio`.
    #[serde(default = "default_mail_from_name")]
    pub from_name: String,
}

fn default_mail_from_name() -> String {
    "audio2".to_string()
}

/// Microsoft sign-in through the `common` authority, so both work/school and
/// personal Microsoft accounts can sign in. Shaped like `GoogleAuthConfig`
/// deliberately — the flow is the same loopback + PKCE exchange and the only
/// real difference is in token verification (see `oidc::verify_microsoft_id_token`).
#[derive(Debug, Clone, Deserialize)]
pub struct MicrosoftAuthConfig {
    /// Master switch, independent of whether credentials are set, so they can be
    /// configured and exercised before the feature goes live for real users.
    #[serde(default)]
    pub enabled: bool,

    /// Comma-separated application (client) IDs accepted as the ID token `aud`.
    pub client_ids: String,

    /// The Entra app registration used server-side for the loopback
    /// authorization-code exchange. The secret stays here rather than in any
    /// client: the app only ever posts a `code`.
    pub desktop_client_id: Option<String>,
    pub desktop_client_secret: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GoogleAuthConfig {
    /// Master switch, independent of whether credentials below are set.
    /// **Off by default** so credentials can be configured and tested ahead
    /// of the public rollout without going live for real users — flip
    /// `AUTH__GOOGLE__ENABLED=true` when it's time, and back to `false` to
    /// pull it again without touching or removing the credentials.
    #[serde(default)]
    pub enabled: bool,

    /// Comma-separated OAuth client IDs accepted as the ID token `aud`
    /// (desktop, iOS, and web clients all listed here as they come online).
    pub client_ids: String,

    /// The "Desktop app" OAuth client used server-side for the loopback
    /// authorization-code exchange (`POST /auth/google` with a `code`).
    /// Not confidential in Google's own threat model for installed apps,
    /// but still kept server-side rather than embedded in the Mac app.
    pub desktop_client_id: Option<String>,
    pub desktop_client_secret: Option<String>,

    /// The "Web application" client, handed to the web console so it can run
    /// Google Identity Services. Served through `/auth/providers` rather than
    /// baked into the bundle at build time — the UI ships inside the backend,
    /// so a build-time value would force self-hosters to rebuild it.
    ///
    /// Public by design: it appears in the page source either way. It must
    /// also be listed in `client_ids`, which is what actually admits a token.
    pub web_client_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppleAuthConfig {
    /// Master switch, independent of whether `client_ids` is set — see
    /// `GoogleAuthConfig::enabled`, same reasoning. **Off by default.**
    #[serde(default)]
    pub enabled: bool,

    /// Comma-separated `aud` values accepted in Apple identity tokens: app
    /// bundle IDs and/or the Services ID, as clients come online.
    pub client_ids: String,

    /// The Services ID the web app hands to Apple's JS SDK as `clientId`.
    /// Public by design, like `GoogleAuthConfig::web_client_id`, and must
    /// also appear in `client_ids`. Unset ⇒ no Apple button on the web.
    pub web_client_id: Option<String>,
}

impl AppConfig {
    /// Load configuration from environment variables (and optional `.env`).
    ///
    /// Naming convention: nested fields are mapped with `__` as separator,
    /// e.g. `SERVER__PORT=8080` sets `server.port`.
    pub fn load() -> anyhow::Result<Self> {
        let cfg = config::Config::builder()
            .add_source(
                config::Environment::default()
                    .separator("__")
                    .try_parsing(true),
            )
            .build()
            .context("failed to build configuration")?;

        cfg.try_deserialize().context("failed to deserialize configuration")
    }
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_session_ttl() -> u64 {
    86400 * 7 // 7 days
}

fn default_refresh_ttl() -> u64 {
    86400 * 90 // 90 days
}

impl AppConfig {
    /// Parsed job-type allowlist for this process's worker loop, or `None`
    /// to claim every job type (the default).
    pub fn worker_job_types(&self) -> Option<Vec<String>> {
        self.worker_job_types
            .as_ref()
            .map(|raw| raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
    }

    /// Jobs run concurrently by this process's worker loop; never zero, since
    /// zero would claim jobs it can never run.
    pub fn worker_max_concurrent_jobs(&self) -> usize {
        self.worker_max_concurrent_jobs.unwrap_or(4).max(1)
    }
}

impl AuthConfig {
    /// Effective access-JWT TTL: `access_ttl_secs` when set, else the
    /// deprecated `session_ttl_secs`.
    pub fn access_ttl(&self) -> u64 {
        self.access_ttl_secs.unwrap_or(self.session_ttl_secs)
    }
}

fn default_true() -> bool {
    true
}

fn default_s3_region() -> String {
    "garage".to_string()
}
