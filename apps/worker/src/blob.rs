use std::path::{Path, PathBuf};

use crate::types::JobError;

#[derive(Debug, Clone)]
pub struct BlobPut {
    pub url: String,
    pub pathname: String,
}

#[derive(Clone)]
pub enum ObjectStore {
    Fs { root: PathBuf },
    Vercel { token: String, api: String },
}

impl ObjectStore {
    pub fn from_env() -> Self {
        if let Ok(token) = std::env::var("BLOB_READ_WRITE_TOKEN")
            && !token.is_empty()
        {
            return Self::Vercel {
                token,
                api: std::env::var("VERCEL_BLOB_API")
                    .unwrap_or_else(|_| "https://blob.vercel-storage.com".into()),
            };
        }
        let root =
            PathBuf::from(std::env::var("STORE_DIR").unwrap_or_else(|_| ".data/blob".into()));
        Self::Fs { root }
    }

    pub async fn put(
        &self,
        client: &reqwest::Client,
        pathname: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<BlobPut, JobError> {
        match self {
            Self::Fs { root } => {
                let dest = root.join(pathname);
                if let Some(parent) = dest.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .map_err(|err| JobError::msg(format!("store mkdir: {err}")))?;
                }
                tokio::fs::write(&dest, &bytes)
                    .await
                    .map_err(|err| JobError::msg(format!("store write: {err}")))?;
                let public = std::env::var("PUBLIC_BLOB_URL")
                    .unwrap_or_else(|_| "http://127.0.0.1:3000/api/blob".into());
                Ok(BlobPut {
                    url: format!("{public}/{pathname}"),
                    pathname: pathname.to_string(),
                })
            }
            Self::Vercel { token, api } => {
                let url = format!("{api}/?pathname={}", urlencoding_pathname(pathname));
                let response = client
                    .put(url)
                    .header("authorization", format!("Bearer {token}"))
                    .header("x-api-version", "7")
                    .header("x-content-type", content_type)
                    .body(bytes)
                    .send()
                    .await
                    .map_err(|err| JobError::msg(format!("blob put: {err}")))?;
                if !response.status().is_success() {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    return Err(JobError::msg(format!("blob put {status}: {text}")));
                }
                let value: serde_json::Value = response
                    .json()
                    .await
                    .map_err(|err| JobError::msg(format!("blob put json: {err}")))?;
                let url = value
                    .get("url")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| JobError::msg("blob put missing url"))?
                    .to_string();
                Ok(BlobPut {
                    url,
                    pathname: pathname.to_string(),
                })
            }
        }
    }

    pub async fn head_length(
        &self,
        client: &reqwest::Client,
        url: &str,
        pathname: &str,
    ) -> Result<Option<u64>, JobError> {
        match self {
            Self::Fs { root } => {
                let dest = root.join(pathname);
                let meta = tokio::fs::metadata(dest)
                    .await
                    .map_err(|err| JobError::msg(format!("store head: {err}")))?;
                Ok(Some(meta.len()))
            }
            Self::Vercel { token, .. } => {
                let response = client
                    .head(url)
                    .header("authorization", format!("Bearer {token}"))
                    .send()
                    .await
                    .map_err(|err| JobError::msg(format!("blob head: {err}")))?;
                if !response.status().is_success() {
                    return Err(JobError::msg(format!("blob head {}", response.status())));
                }
                Ok(response
                    .headers()
                    .get(reqwest::header::CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok()))
            }
        }
    }

    pub fn local_path(&self, pathname: &str) -> Option<PathBuf> {
        match self {
            Self::Fs { root } => Some(root.join(pathname)),
            Self::Vercel { .. } => None,
        }
    }

    pub async fn delete(&self, pathname: &str) -> Result<(), JobError> {
        match self {
            Self::Fs { root } => {
                let dest = root.join(pathname);
                match tokio::fs::remove_file(dest).await {
                    Ok(()) => Ok(()),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(err) => Err(JobError::msg(format!("store delete: {err}"))),
                }
            }
            Self::Vercel { .. } => Ok(()),
        }
    }
}

fn urlencoding_pathname(pathname: &str) -> String {
    pathname
        .split('/')
        .map(|part| {
            let mut encoded = String::new();
            for byte in part.bytes() {
                match byte {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                        encoded.push(byte as char);
                    }
                    _ => encoded.push_str(&format!("%{byte:02X}")),
                }
            }
            encoded
        })
        .collect::<Vec<_>>()
        .join("/")
}

pub async fn download_source(
    client: &reqwest::Client,
    store: &ObjectStore,
    url: &str,
    pathname: &str,
    dest: &Path,
) -> Result<(), JobError> {
    if let Some(local) = store.local_path(pathname)
        && local.exists()
    {
        tokio::fs::copy(local, dest)
            .await
            .map_err(|err| JobError::msg(format!("copy source: {err}")))?;
        return Ok(());
    }
    if let Some(stripped) = url.strip_prefix("file://") {
        tokio::fs::copy(stripped, dest)
            .await
            .map_err(|err| JobError::msg(format!("copy file source: {err}")))?;
        return Ok(());
    }
    let mut request = client.get(url);
    if let ObjectStore::Vercel { token, .. } = store {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = request
        .send()
        .await
        .map_err(|err| JobError::msg(format!("download source: {err}")))?;
    if !response.status().is_success() {
        return Err(JobError::msg(format!(
            "download source {}",
            response.status()
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|err| JobError::msg(format!("download body: {err}")))?;
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|err| JobError::msg(format!("source mkdir: {err}")))?;
    }
    tokio::fs::write(dest, bytes)
        .await
        .map_err(|err| JobError::msg(format!("write source: {err}")))?;
    Ok(())
}
