// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::app::StorageConfig;
use anyhow::Context;
use aws_config::Region;
use aws_credential_types::Credentials;
use aws_sdk_s3::Client;
use aws_sdk_s3::config::{BehaviorVersion, Builder as S3Builder, RequestChecksumCalculation};
use aws_sdk_s3::primitives::ByteStream;
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;

pub mod models;

/// Object key for media owned by a family: `f/{family_id}/{suffix}`.
///
/// Scoped by the *owning* family, never by an item's visibility: visibility is
/// mutable (private ⇄ family) while the key must not be, or sharing an item
/// would mean copying the object. Content-addressed artwork that is
/// deliberately deduplicated across families stays outside this prefix under
/// `shared/` — see `podcasts::store_image_from_url`.
pub fn family_key(family_id: uuid::Uuid, suffix: impl AsRef<str>) -> String {
    format!("f/{family_id}/{}", suffix.as_ref())
}

/// S3-compatible object storage (Garage, MinIO, AWS S3, ...).
///
/// Two clients are held because SigV4 signatures cover the Host header:
/// `client` targets the in-network endpoint for API calls, while
/// `presign_client` targets the browser-reachable endpoint so presigned
/// URLs validate when fetched directly by the player.
#[derive(Clone)]
pub struct ObjectStore {
    client: Client,
    presign_client: Client,
    bucket: String,
}

impl ObjectStore {
    /// Bucket name for provenance in `media_objects.bucket`.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// Upload bytes to the given object key.
    pub async fn put(&self, key: &str, body: Bytes, content_type: &str) -> anyhow::Result<()> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(body.into())
            .content_type(content_type)
            .send()
            .await
            .context("object store put failed")?;
        Ok(())
    }

    /// Upload a file from disk without loading the full payload into memory.
    pub async fn put_path(&self, key: &str, path: &Path, content_type: &str) -> anyhow::Result<()> {
        let body = ByteStream::from_path(path.to_path_buf())
            .await
            .context("create byte stream from path")?;

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(body)
            .content_type(content_type)
            .send()
            .await
            .context("object store put path failed")?;

        Ok(())
    }

    /// Download an object and return its bytes.
    pub async fn get(&self, key: &str) -> anyhow::Result<Bytes> {
        let output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .context("object store get failed")?;

        let bytes = output
            .body
            .collect()
            .await
            .context("read object body")?
            .into_bytes();

        Ok(bytes)
    }

    /// Stream an object into a temporary file, for reading what needs a whole
    /// file on disk (tags) without holding it in memory — see `sha256_hex`.
    pub async fn download_to_temp(&self, key: &str) -> anyhow::Result<tempfile::NamedTempFile> {
        use tokio::io::AsyncWriteExt;
        let mut output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .context("object store get failed")?;
        let temp = tempfile::NamedTempFile::new().context("create temp file")?;
        let mut file = tokio::fs::File::from_std(temp.reopen().context("open temp file")?);
        while let Some(chunk) = output.body.next().await {
            file.write_all(&chunk.context("read object body")?).await.context("write temp file")?;
        }
        file.flush().await.context("flush temp file")?;
        Ok(temp)
    }

    /// SHA-256 of an object's stored bytes, hashed as the body streams in, so
    /// memory stays at one network chunk whatever the object size. `get` holds
    /// the whole file — twice, briefly, since `collect().into_bytes()` copies
    /// the segmented body into one contiguous buffer — and hashing a library
    /// of 50–120 MB audiobooks that way is what pinned the canary at 1.5 GB
    /// RSS and starved the CI build sharing its 4 GB host.
    pub async fn sha256_hex(&self, key: &str) -> anyhow::Result<String> {
        let mut output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .context("object store get failed")?;

        let mut hasher = Sha256::new();
        while let Some(chunk) = output.body.next().await {
            hasher.update(&chunk.context("read object body")?);
        }
        Ok(format!("{:x}", hasher.finalize()))
    }

    /// Delete an object by key.
    pub async fn delete(&self, key: &str) -> anyhow::Result<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .context("object store delete failed")?;
        Ok(())
    }

    /// Every object in the bucket, as `(key, last_modified_epoch_secs)`.
    ///
    /// Paginated to exhaustion: `list_objects_v2` caps a page at 1000 keys and
    /// signals more with a continuation token, so stopping at the first page
    /// would quietly under-report a bucket of any real size — and a
    /// reconciliation that thinks the bucket is smaller than it is would draw
    /// exactly the wrong conclusion about what is missing.
    ///
    /// The timestamp is the caller's only safety net: an object uploaded
    /// through a presigned PUT has no database row until `from-uploads`
    /// attaches it, so "not in the database" cannot mean "safe to delete"
    /// without also meaning "and old enough that no upload is still running".
    pub async fn list_all(&self) -> anyhow::Result<Vec<(String, i64)>> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut req = self.client.list_objects_v2().bucket(&self.bucket);
            if let Some(t) = token {
                req = req.continuation_token(t);
            }
            let page = req.send().await.context("object store list failed")?;
            for obj in page.contents() {
                if let Some(key) = obj.key() {
                    let modified = obj.last_modified().map(|t| t.secs()).unwrap_or(0);
                    out.push((key.to_string(), modified));
                }
            }
            match page.next_continuation_token() {
                Some(t) => token = Some(t.to_string()),
                None => break,
            }
        }
        Ok(out)
    }

    /// Generate a presigned GET URL valid for `expires_in_secs` seconds,
    /// signed against the browser-reachable endpoint.
    pub async fn presigned_get(&self, key: &str, expires_in_secs: u64) -> anyhow::Result<String> {
        use aws_sdk_s3::presigning::PresigningConfig;

        let config = PresigningConfig::expires_in(Duration::from_secs(expires_in_secs))
            .context("invalid presigning duration")?;

        let presigned = self
            .presign_client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(config)
            .await
            .context("presigning failed")?;

        Ok(presigned.uri().to_string())
    }

    /// Generate a presigned PUT URL valid for `expires_in_secs` seconds,
    /// signed against the browser-reachable endpoint.
    ///
    /// `content_type` is part of the signature, so the client's PUT must send
    /// exactly the same `Content-Type` header or the request is rejected with
    /// a signature mismatch.
    pub async fn presigned_put(
        &self,
        key: &str,
        content_type: &str,
        expires_in_secs: u64,
    ) -> anyhow::Result<String> {
        use aws_sdk_s3::presigning::PresigningConfig;

        let config = PresigningConfig::expires_in(Duration::from_secs(expires_in_secs))
            .context("invalid presigning duration")?;

        let presigned = self
            .presign_client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .presigned(config)
            .await
            .context("presigning put failed")?;

        Ok(presigned.uri().to_string())
    }

    /// Size and content type of an object, or `None` when it does not exist.
    pub async fn head_object(&self, key: &str) -> anyhow::Result<Option<(i64, String)>> {
        match self.client.head_object().bucket(&self.bucket).key(key).send().await {
            Ok(output) => Ok(Some((
                output.content_length().unwrap_or_default(),
                output
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_string(),
            ))),
            Err(err) => {
                // A missing object is an expected answer here (the client
                // never completed its PUT), not a storage failure.
                if err
                    .as_service_error()
                    .map(|e| e.is_not_found())
                    .unwrap_or(false)
                {
                    Ok(None)
                } else {
                    Err(anyhow::Error::from(err).context("object store head failed"))
                }
            }
        }
    }

    /// Check whether the configured bucket exists and is accessible.
    pub async fn head_bucket(&self) -> anyhow::Result<()> {
        self.client
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .context("bucket not accessible")?;
        Ok(())
    }

    /// Apply a permissive GET/HEAD CORS policy so browsers can fetch
    /// presigned URLs from other origins. Playback via media elements
    /// works without this; it future-proofs fetch-based consumers.
    async fn ensure_cors(&self) {
        use aws_sdk_s3::types::{CorsConfiguration, CorsRule};

        let rule = CorsRule::builder()
            .allowed_methods("GET")
            .allowed_methods("HEAD")
            .allowed_origins("*")
            .allowed_headers("*")
            .expose_headers("Content-Range")
            .expose_headers("Accept-Ranges")
            .expose_headers("Content-Length")
            .expose_headers("ETag")
            .build();

        let config = match rule
            .map_err(anyhow::Error::from)
            .and_then(|r| CorsConfiguration::builder().cors_rules(r).build().map_err(Into::into))
        {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!(error = %err, "failed to build CORS configuration");
                return;
            }
        };

        if let Err(err) = self
            .client
            .put_bucket_cors()
            .bucket(&self.bucket)
            .cors_configuration(config)
            .send()
            .await
        {
            tracing::warn!(bucket = %self.bucket, error = %err, "failed to set bucket CORS policy");
        }
    }
}

/// Build an [`ObjectStore`] from configuration.
pub async fn connect(cfg: &StorageConfig) -> anyhow::Result<ObjectStore> {
    let public_endpoint = cfg.public_endpoint.as_deref().unwrap_or(&cfg.endpoint);

    let client = build_client(cfg, &cfg.endpoint);
    let presign_client = build_client(cfg, public_endpoint);

    let store = ObjectStore {
        client,
        presign_client,
        bucket: cfg.bucket.clone(),
    };

    ensure_bucket_with_retry(&store).await;

    if cfg.set_cors {
        store.ensure_cors().await;
    }

    Ok(store)
}

fn build_client(cfg: &StorageConfig, endpoint: &str) -> Client {
    let creds = Credentials::new(
        &cfg.access_key,
        &cfg.secret_key,
        None,
        None,
        "audio2-static",
    );

    let s3_config = S3Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .endpoint_url(endpoint)
        .region(Region::new(cfg.region.clone()))
        .credentials_provider(creds)
        .force_path_style(cfg.path_style)
        // Garage and MinIO do not support the AWS chunked-transfer +
        // trailing checksum encoding that the SDK uses by default for
        // large files. WhenRequired disables automatic checksum injection
        // for PutObject, making large uploads compatible.
        .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
        .build();

    Client::from_conf(s3_config)
}

/// Wait for the bucket to become reachable, creating it if needed.
///
/// Never fails startup: storage problems are surfaced by the setup
/// wizard self-test instead of crash-looping the whole app.
async fn ensure_bucket_with_retry(store: &ObjectStore) {
    const ATTEMPTS: u32 = 15;
    const DELAY: Duration = Duration::from_secs(2);

    for attempt in 1..=ATTEMPTS {
        match store.head_bucket().await {
            Ok(_) => {
                tracing::info!(bucket = %store.bucket, "object storage bucket ready");
                return;
            }
            Err(err) if attempt < ATTEMPTS => {
                tracing::debug!(
                    bucket = %store.bucket,
                    attempt,
                    error = %err,
                    "object storage not ready yet, retrying"
                );
                tokio::time::sleep(DELAY).await;
            }
            Err(_) => {}
        }
    }

    tracing::info!(bucket = %store.bucket, "bucket not reachable, attempting to create it");
    match store.client.create_bucket().bucket(&store.bucket).send().await {
        Ok(_) => {
            tracing::info!(bucket = %store.bucket, "object storage bucket created");
        }
        Err(err) => {
            tracing::warn!(
                bucket = %store.bucket,
                error = %err,
                "could not reach or create the storage bucket; on Garage run \
                 `garage bucket create <name>` and `garage bucket allow \
                 --read --write --owner <name> --key <key>`; storage \
                 operations will fail until this is resolved"
            );
        }
    }
}
