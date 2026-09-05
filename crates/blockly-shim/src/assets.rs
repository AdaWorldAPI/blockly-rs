//! Raw assets — the costume bitmaps/vectors and sound files an sb3 carries.
//!
//! These are NOT program data. A costume's NAME is a register entry (the
//! COSTUME codebook, menu 14) and a program refers to it by index; the
//! bytes behind the name never enter a node, a register, or a pool. They
//! cross the intake boundary exactly once and land in a key-value store,
//! **content-addressed by sb3's own `md5ext`** (`<md5>.<ext>`), so the same
//! file referenced by ten sprites or ten projects is stored once.
//!
//! The store is a seam: [`AssetStore`] is the whole contract, [`MemoryStore`]
//! is the in-process implementation every test uses, and [`S3Store`]
//! (feature `s3`) is the deployment one — an S3-compatible bucket (Tigris on
//! Railway) reached through the standard `AWS_*` variables. A Lance side
//! table keyed by the same `md5ext` is the natural index over the bucket
//! when a consumer needs to query assets rather than fetch them; it is not
//! built here (it would pull the lance dependency tree into the membrane),
//! and the key shape is what makes it a later, additive step.

use std::collections::HashMap;
use std::sync::RwLock;

/// Why an asset could not be stored or read.
#[derive(Debug)]
pub enum AssetError {
    /// The backing store refused or failed; the message is the backend's.
    Store(String),
    /// Configuration is missing — for [`S3Store::from_env`], which variable.
    Config(&'static str),
}

impl core::fmt::Display for AssetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Store(m) => write!(f, "asset store: {m}"),
            Self::Config(k) => write!(f, "asset store: missing {k}"),
        }
    }
}

impl std::error::Error for AssetError {}

/// A content-addressed blob store. Keys are sb3 `md5ext` strings.
pub trait AssetStore {
    /// Whether `key` is already stored — the dedup check the intake makes
    /// BEFORE uploading, so a re-import of a project moves no bytes.
    ///
    /// # Errors
    ///
    /// [`AssetError::Store`] when the backend cannot answer.
    fn has(&self, key: &str) -> Result<bool, AssetError>;
    /// Store `bytes` under `key`. Idempotent: the same key holds the same
    /// bytes by construction (the key IS their digest).
    ///
    /// # Errors
    ///
    /// [`AssetError::Store`].
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), AssetError>;
    /// Fetch the bytes under `key`, if stored.
    ///
    /// # Errors
    ///
    /// [`AssetError::Store`].
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, AssetError>;
}

/// An in-process store: tests, and a single-process demo.
#[derive(Debug, Default)]
pub struct MemoryStore {
    blobs: RwLock<HashMap<String, Vec<u8>>>,
}

impl MemoryStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many distinct keys are stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.blobs.read().map_or(0, |b| b.len())
    }

    /// Whether nothing is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl AssetStore for MemoryStore {
    fn has(&self, key: &str) -> Result<bool, AssetError> {
        Ok(self
            .blobs
            .read()
            .map_err(|e| AssetError::Store(e.to_string()))?
            .contains_key(key))
    }
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), AssetError> {
        self.blobs
            .write()
            .map_err(|e| AssetError::Store(e.to_string()))?
            .insert(key.to_string(), bytes.to_vec());
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, AssetError> {
        Ok(self
            .blobs
            .read()
            .map_err(|e| AssetError::Store(e.to_string()))?
            .get(key)
            .cloned())
    }
}

/// What one archive intake did — counts, so a caller can log "moved N
/// bytes" rather than assume.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IngestReport {
    /// Assets the project references (costumes + sounds, all targets).
    pub referenced: usize,
    /// Of those, files present in the archive.
    pub present: usize,
    /// Files actually uploaded (not already in the store).
    pub stored: usize,
    /// Bytes uploaded.
    pub bytes_stored: u64,
}

/// Errors of a whole-archive intake.
#[cfg(feature = "sb3-archive")]
#[derive(Debug)]
pub enum IngestError {
    /// The bytes are not a readable zip, or `project.json` is missing.
    Archive(String),
    /// `project.json` did not parse — see [`crate::sb3::Sb3Error`].
    Project(crate::sb3::Sb3Error),
    /// The store failed.
    Store(AssetError),
}

#[cfg(feature = "sb3-archive")]
impl core::fmt::Display for IngestError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Archive(m) => write!(f, "sb3 archive: {m}"),
            Self::Project(e) => write!(f, "sb3 project.json: {e}"),
            Self::Store(e) => write!(f, "{e}"),
        }
    }
}

#[cfg(feature = "sb3-archive")]
impl std::error::Error for IngestError {}

/// Read a whole `.sb3` archive: parse `project.json` and move every
/// referenced asset file into `store` under its `md5ext` — once.
///
/// This is the ONE-TIME intake boundary. After it returns, the project is
/// `BlockRecord` trees (cast to nodes + registers by the caller) and the
/// files are in the store; nothing downstream reads the archive or the
/// JSON again.
///
/// An asset the project references but the archive lacks is counted, not
/// an error: Scratch projects in the wild are missing files, and a program
/// that names a missing costume is still a program.
///
/// # Errors
///
/// See [`IngestError`].
#[cfg(feature = "sb3-archive")]
pub fn ingest_sb3(
    archive: &[u8],
    store: &dyn AssetStore,
) -> Result<(crate::sb3::Sb3Project, IngestReport), IngestError> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))
        .map_err(|e| IngestError::Archive(e.to_string()))?;
    let mut json = String::new();
    zip.by_name("project.json")
        .map_err(|e| IngestError::Archive(format!("project.json: {e}")))?
        .read_to_string(&mut json)
        .map_err(|e| IngestError::Archive(e.to_string()))?;
    let project = crate::sb3::from_project_json(&json).map_err(IngestError::Project)?;

    let mut report = IngestReport::default();
    let mut seen: Vec<&str> = Vec::new();
    for t in &project.targets {
        for a in t.costume_assets.iter().chain(&t.sound_assets) {
            report.referenced += 1;
            if seen.contains(&a.md5ext.as_str()) {
                continue;
            }
            seen.push(&a.md5ext);
            let Ok(mut file) = zip.by_name(&a.md5ext) else {
                continue;
            };
            report.present += 1;
            if store.has(&a.md5ext).map_err(IngestError::Store)? {
                continue;
            }
            let mut bytes = Vec::with_capacity(usize::try_from(file.size()).unwrap_or(0));
            file.read_to_end(&mut bytes)
                .map_err(|e| IngestError::Archive(e.to_string()))?;
            store.put(&a.md5ext, &bytes).map_err(IngestError::Store)?;
            report.stored += 1;
            report.bytes_stored += bytes.len() as u64;
        }
    }
    Ok((project, report))
}

/// An S3-compatible bucket as the asset store (feature `s3`).
///
/// Configured from the environment a Railway service already has for
/// Tigris: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_ENDPOINT_URL`,
/// `AWS_S3_BUCKET_NAME`, and optionally `AWS_DEFAULT_REGION` (default
/// `auto`) — the same NAMES `lance-graph-hydrate::env` reads, so one
/// deployment serves both. Values are read and handed to the client; they
/// are never logged or echoed. Keys are stored under `BLOCKLY_ASSET_PREFIX`
/// (default `sb3-assets/`).
#[cfg(feature = "s3")]
pub struct S3Store {
    inner: std::sync::Arc<dyn object_store::ObjectStore>,
    prefix: String,
    rt: tokio::runtime::Runtime,
}

#[cfg(feature = "s3")]
impl std::fmt::Debug for S3Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Store")
            .field("prefix", &self.prefix)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "s3")]
impl S3Store {
    /// Read one variable, stripping the quotes some exporters wrap around a
    /// value (a quoted credential authenticates as garbage).
    fn env(k: &'static str) -> Option<String> {
        std::env::var(k)
            .ok()
            .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
            .filter(|v| !v.is_empty())
    }

    /// Build the store from the environment.
    ///
    /// # Errors
    ///
    /// [`AssetError::Config`] naming the first missing variable;
    /// [`AssetError::Store`] if the client cannot be built.
    pub fn from_env() -> Result<Self, AssetError> {
        use object_store::aws::AmazonS3Builder;
        let key = Self::env("AWS_ACCESS_KEY_ID").ok_or(AssetError::Config("AWS_ACCESS_KEY_ID"))?;
        let secret = Self::env("AWS_SECRET_ACCESS_KEY")
            .ok_or(AssetError::Config("AWS_SECRET_ACCESS_KEY"))?;
        let endpoint =
            Self::env("AWS_ENDPOINT_URL").ok_or(AssetError::Config("AWS_ENDPOINT_URL"))?;
        let bucket =
            Self::env("AWS_S3_BUCKET_NAME").ok_or(AssetError::Config("AWS_S3_BUCKET_NAME"))?;
        let region = Self::env("AWS_DEFAULT_REGION").unwrap_or_else(|| "auto".into());
        let prefix = Self::env("BLOCKLY_ASSET_PREFIX").unwrap_or_else(|| "sb3-assets/".into());
        let store = AmazonS3Builder::new()
            .with_access_key_id(key)
            .with_secret_access_key(secret)
            .with_endpoint(endpoint)
            .with_bucket_name(bucket)
            .with_region(region)
            .with_virtual_hosted_style_request(false)
            .build()
            .map_err(|e| AssetError::Store(e.to_string()))?;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| AssetError::Store(e.to_string()))?;
        Ok(Self {
            inner: std::sync::Arc::new(store),
            prefix,
            rt,
        })
    }

    fn path(&self, key: &str) -> object_store::path::Path {
        object_store::path::Path::from(format!("{}{}", self.prefix, key))
    }
}

#[cfg(feature = "s3")]
impl AssetStore for S3Store {
    fn has(&self, key: &str) -> Result<bool, AssetError> {
        use object_store::ObjectStoreExt as _;
        let p = self.path(key);
        match self.rt.block_on(self.inner.head(&p)) {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(e) => Err(AssetError::Store(e.to_string())),
        }
    }
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), AssetError> {
        use object_store::ObjectStoreExt as _;
        let p = self.path(key);
        let payload = object_store::PutPayload::from(bytes.to_vec());
        self.rt
            .block_on(self.inner.put(&p, payload))
            .map(|_| ())
            .map_err(|e| AssetError::Store(e.to_string()))
    }
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, AssetError> {
        use object_store::ObjectStoreExt as _;
        let p = self.path(key);
        match self.rt.block_on(async {
            let r = self.inner.get(&p).await?;
            r.bytes().await
        }) {
            Ok(b) => Ok(Some(b.to_vec())),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(e) => Err(AssetError::Store(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_memory_store_is_content_keyed_and_idempotent() {
        let s = MemoryStore::new();
        assert!(!s.has("ab.png").unwrap());
        s.put("ab.png", b"PNG").unwrap();
        assert!(s.has("ab.png").unwrap());
        s.put("ab.png", b"PNG").unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s.get("ab.png").unwrap().as_deref(), Some(&b"PNG"[..]));
        assert_eq!(s.get("zz.wav").unwrap(), None);
    }

    /// The intake moves each referenced file ONCE, skips what the store
    /// already holds, and counts a referenced-but-absent file without
    /// failing — measured on a zip built here, not assumed.
    #[cfg(feature = "sb3-archive")]
    #[test]
    fn an_archive_intake_stores_each_asset_once_and_tolerates_a_missing_file() {
        use std::io::Write;
        let project = r#"{"targets":[
          {"isStage":true,"name":"Stage","variables":{},"lists":{},"broadcasts":{},
           "blocks":{},"costumes":[{"name":"backdrop1","md5ext":"aa.svg","dataFormat":"svg"}],"sounds":[]},
          {"isStage":false,"name":"Cat","variables":{},"lists":{},"broadcasts":{},
           "blocks":{},
           "costumes":[{"name":"c1","md5ext":"bb.png","dataFormat":"png"},
                       {"name":"c2","md5ext":"bb.png","dataFormat":"png"},
                       {"name":"lost","md5ext":"zz.png","dataFormat":"png"}],
           "sounds":[{"name":"pop","md5ext":"cc.wav","dataFormat":"wav"}]}
        ]}"#;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            for (name, bytes) in [
                ("project.json", project.as_bytes()),
                ("aa.svg", &b"<svg/>"[..]),
                ("bb.png", &b"PNGPNG"[..]),
                ("cc.wav", &b"RIFF"[..]),
            ] {
                w.start_file(name, opts).unwrap();
                w.write_all(bytes).unwrap();
            }
            w.finish().unwrap();
        }
        let archive = buf.into_inner();
        let store = MemoryStore::new();
        // Pre-seed one file: it must be skipped, not re-uploaded.
        store.put("cc.wav", b"RIFF").unwrap();

        let (p, r) = ingest_sb3(&archive, &store).unwrap();
        assert_eq!(p.targets.len(), 2);
        assert_eq!(p.targets[1].costume_assets[0].md5ext, "bb.png");
        assert_eq!(
            r,
            IngestReport {
                referenced: 5,
                present: 3,
                stored: 2,
                bytes_stored: 6 + 6,
            }
        );
        assert_eq!(store.len(), 3, "aa, bb, cc — bb once despite two costumes");

        // A second intake of the same archive moves nothing.
        let (_, again) = ingest_sb3(&archive, &store).unwrap();
        assert_eq!(again.stored, 0);
        assert_eq!(again.bytes_stored, 0);
        assert_eq!(again.present, 3);
    }
}
