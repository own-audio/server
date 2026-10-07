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
use std::path::{Path, PathBuf};
use std::time::Duration;

pub mod links;
pub mod models;

pub use links::MediaLinks;

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
///
/// Or a folder on the server's own disk (`STORAGE__KIND=local`): the same
/// operations on files under one root, with links to the server's media
/// route standing in for presigned URLs. Callers cannot tell the two apart.
#[derive(Clone)]
pub struct ObjectStore {
    client: Client,
    presign_client: Client,
    bucket: String,
    /// `Some` for local storage: objects are files under this folder.
    local: Option<PathBuf>,
    /// Server-signed links, used instead of presigned URLs for local storage
    /// and for S3 with `STORAGE__PROXY`.
    links: Option<MediaLinks>,
}

/// An object opened for streaming, possibly a byte range of it.
pub struct OpenedObject {
    pub total: u64,
    pub start: u64,
    pub end: u64,
    pub content_type: String,
    pub body: axum::body::Body,
}

impl ObjectStore {
    /// Bucket name for provenance in `media_objects.bucket`; `local` for the
    /// local backend.
    pub fn bucket(&self) -> &str {
        if self.local.is_some() { "local" } else { &self.bucket }
    }

    /// The links this store signs, when it signs its own. The media route
    /// verifies requests with them.
    pub fn media_links(&self) -> Option<&MediaLinks> {
        self.links.as_ref()
    }

    /// The file behind `key` in local storage. Keys are paths the server
    /// built itself, but they also arrive in media links and upload
    /// requests, so anything that could leave the root is refused.
    fn local_path(&self, key: &str) -> anyhow::Result<PathBuf> {
        let root = self.local.as_ref().context("not local storage")?;
        let rel = Path::new(key);
        let safe = !key.is_empty()
            && rel.components().all(|c| matches!(c, std::path::Component::Normal(_)));
        anyhow::ensure!(safe, "invalid object key");
        Ok(root.join(rel))
    }

    /// A temporary file next to the stored objects, so finishing an upload is
    /// a rename on the same disk rather than a copy.
    pub fn new_temp(&self) -> anyhow::Result<tempfile::NamedTempFile> {
        match &self.local {
            Some(root) => {
                let dir = root.join(".tmp");
                std::fs::create_dir_all(&dir).context("create upload temp dir")?;
                tempfile::NamedTempFile::new_in(dir).context("create temp file")
            }
            None => tempfile::NamedTempFile::new().context("create temp file"),
        }
    }

    /// Store a finished temporary file under `key`.
    pub async fn put_temp(&self, key: &str, temp: tempfile::NamedTempFile, content_type: &str) -> anyhow::Result<()> {
        if self.local.is_some() {
            self.persist_local(key, temp).await
        } else {
            self.put_path(key, temp.path(), content_type).await
        }
    }

    /// Move a finished temporary file into place under `key` (local storage).
    async fn persist_local(&self, key: &str, temp: tempfile::NamedTempFile) -> anyhow::Result<()> {
        let dest = self.local_path(key)?;
        if let Some(dir) = dest.parent() {
            tokio::fs::create_dir_all(dir).await.context("create object folder")?;
        }
        match temp.persist(&dest) {
            Ok(_) => Ok(()),
            // Another filesystem after all (a temp dir on a different mount).
            Err(e) => {
                tokio::fs::copy(e.file.path(), &dest).await.context("copy upload into storage")?;
                Ok(())
            }
        }
    }

    /// Upload bytes to the given object key.
    pub async fn put(&self, key: &str, body: Bytes, content_type: &str) -> anyhow::Result<()> {
        if self.local.is_some() {
            let mut temp = self.new_temp()?;
            std::io::Write::write_all(&mut temp, &body).context("write object")?;
            return self.persist_local(key, temp).await;
        }
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
        if self.local.is_some() {
            let temp = self.new_temp()?;
            tokio::fs::copy(path, temp.path()).await.context("copy into storage")?;
            return self.persist_local(key, temp).await;
        }
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
        if self.local.is_some() {
            let data = tokio::fs::read(self.local_path(key)?).await.context("object store get failed")?;
            return Ok(Bytes::from(data));
        }
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
        if self.local.is_some() {
            let temp = tempfile::NamedTempFile::new().context("create temp file")?;
            tokio::fs::copy(self.local_path(key)?, temp.path()).await.context("object store get failed")?;
            return Ok(temp);
        }
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
        if self.local.is_some() {
            use tokio::io::AsyncReadExt;
            let mut file = tokio::fs::File::open(self.local_path(key)?).await.context("object store get failed")?;
            let mut hasher = Sha256::new();
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = file.read(&mut buf).await.context("read object")?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            return Ok(format!("{:x}", hasher.finalize()));
        }
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
        if self.local.is_some() {
            return match tokio::fs::remove_file(self.local_path(key)?).await {
                Ok(()) => Ok(()),
                // S3 deletes of a missing key succeed too.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(anyhow::Error::from(e).context("object store delete failed")),
            };
        }
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
        if let Some(root) = &self.local {
            let mut out = Vec::new();
            let mut dirs = vec![root.clone()];
            while let Some(dir) = dirs.pop() {
                let mut entries = match tokio::fs::read_dir(&dir).await {
                    Ok(e) => e,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(anyhow::Error::from(e).context("object store list failed")),
                };
                while let Some(entry) = entries.next_entry().await.context("object store list failed")? {
                    let path = entry.path();
                    let meta = entry.metadata().await.context("object store list failed")?;
                    if meta.is_dir() {
                        if path.file_name().is_some_and(|n| n != ".tmp") {
                            dirs.push(path);
                        }
                    } else if let Ok(rel) = path.strip_prefix(root) {
                        let modified = meta
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0);
                        out.push((rel.to_string_lossy().replace('\\', "/"), modified));
                    }
                }
            }
            return Ok(out);
        }
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

        if let Some(links) = &self.links {
            return Ok(links.link("GET", key, expires_in_secs));
        }

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

        if let Some(links) = &self.links {
            return Ok(links.link("PUT", key, expires_in_secs));
        }

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
        if self.local.is_some() {
            return match tokio::fs::metadata(self.local_path(key)?).await {
                Ok(meta) if meta.is_file() => Ok(Some((meta.len() as i64, guess_content_type(key)))),
                Ok(_) => Ok(None),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(anyhow::Error::from(e).context("object store head failed")),
            };
        }
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
        if let Some(root) = &self.local {
            tokio::fs::create_dir_all(root).await.context("storage folder not writable")?;
            return Ok(());
        }
        self.client
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .context("bucket not accessible")?;
        Ok(())
    }

    /// Open an object for streaming, optionally a byte range of it
    /// (`start`, inclusive `end`; an open end means to the last byte).
    /// `None` when the object does not exist or the range starts past its
    /// end. Memory stays at one chunk whatever the file size.
    pub async fn open(&self, key: &str, range: Option<(u64, Option<u64>)>) -> anyhow::Result<Option<OpenedObject>> {
        let Some((total, content_type)) = self.head_object(key).await? else {
            return Ok(None);
        };
        let total = total.max(0) as u64;
        let (start, end) = match range {
            Some((start, end)) => {
                if start >= total {
                    return Ok(None);
                }
                (start, end.map(|e| e.min(total - 1)).unwrap_or(total - 1))
            }
            None => (0, total.saturating_sub(1)),
        };
        let len = if total == 0 { 0 } else { end - start + 1 };

        let body = if self.local.is_some() {
            use tokio::io::{AsyncReadExt, AsyncSeekExt};
            let mut file = tokio::fs::File::open(self.local_path(key)?).await.context("open object")?;
            file.seek(std::io::SeekFrom::Start(start)).await.context("seek object")?;
            axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file.take(len)))
        } else {
            let mut req = self.client.get_object().bucket(&self.bucket).key(key);
            if range.is_some() {
                req = req.range(format!("bytes={start}-{end}"));
            }
            let output = req.send().await.context("object store get failed")?;
            axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(output.body.into_async_read()))
        };
        Ok(Some(OpenedObject { total, start, end, content_type, body }))
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
///
/// `links` signs the server's own media links; they are used for local
/// storage, and for S3 when `cfg.proxy` is set.
pub async fn connect(cfg: &StorageConfig, links: MediaLinks) -> anyhow::Result<ObjectStore> {
    let public_endpoint = cfg.public_endpoint.as_deref().unwrap_or(&cfg.endpoint);
    let local = match cfg.kind.as_str() {
        "local" => Some(PathBuf::from(cfg.path.as_deref().unwrap_or("/data/media"))),
        "s3" => None,
        other => anyhow::bail!("STORAGE__KIND must be `s3` or `local`, not `{other}`"),
    };

    // The S3 clients exist for both kinds (they are cheap and never called
    // for local storage), so the struct needs no second shape.
    let client = build_client(cfg, if cfg.endpoint.is_empty() { "http://unused.invalid" } else { &cfg.endpoint });
    let presign_client = build_client(cfg, if public_endpoint.is_empty() { "http://unused.invalid" } else { public_endpoint });

    let store = ObjectStore {
        client,
        presign_client,
        bucket: cfg.bucket.clone(),
        links: (local.is_some() || cfg.proxy).then_some(links),
        local,
    };

    if let Some(root) = &store.local {
        tokio::fs::create_dir_all(root)
            .await
            .with_context(|| format!("create the storage folder {}", root.display()))?;
        tracing::info!(path = %root.display(), "local storage ready");
        return Ok(store);
    }

    ensure_bucket_with_retry(&store).await;

    if cfg.set_cors {
        store.ensure_cors().await;
    }

    Ok(store)
}

/// Content type for a stored object from its key's extension; local storage
/// keeps no metadata of its own.
fn guess_content_type(key: &str) -> String {
    let ext = key.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        // mime_guess calls these `audio/mp4a-latm` and `audio/x-m4b`, which
        // Safari refuses to play.
        "m4a" | "m4b" | "mp4" | "aac" => "audio/mp4".to_string(),
        "opus" => "audio/ogg".to_string(),
        _ => mime_guess::from_path(key).first_or_octet_stream().essence_str().to_string(),
    }
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
