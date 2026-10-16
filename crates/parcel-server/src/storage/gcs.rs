//! Google Cloud Storage blob store over the JSON API.
//!
//! Three calls are all parcel needs (media upload, media download, delete),
//! so this speaks the REST API directly with the shared `reqwest` client
//! rather than pulling in a full SDK. Pointing `STORAGE_EMULATOR_HOST` at
//! fake-gcs-server exercises the same code path locally; with no credentials
//! configured under the emulator, requests go unauthenticated.

use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use reqwest::StatusCode;

use super::google_auth::{redact, TokenProvider};
use super::{BlobStore, StorageError};

/// Object names go in a path segment (or query value), so everything but
/// unreserved characters is encoded — including `/`.
const OBJECT_NAME: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

pub struct GcsBlobStore {
    http: reqwest::Client,
    base_url: String,
    bucket: String,
    prefix: String,
    auth: Option<TokenProvider>,
}

impl GcsBlobStore {
    pub fn new(
        http: reqwest::Client,
        bucket: String,
        prefix: String,
        emulator_host: Option<String>,
        credentials_b64: Option<&str>,
    ) -> anyhow::Result<Self> {
        let base_url = match &emulator_host {
            Some(host) if host.starts_with("http://") || host.starts_with("https://") => {
                host.trim_end_matches('/').to_string()
            }
            Some(host) => format!("http://{}", host.trim_end_matches('/')),
            None => "https://storage.googleapis.com".to_string(),
        };
        let auth = match (credentials_b64, emulator_host.is_some()) {
            (Some(b64), _) => Some(TokenProvider::from_service_account(http.clone(), b64)?),
            (None, true) => None,
            (None, false) => Some(TokenProvider::metadata(http.clone())),
        };
        let prefix = prefix.trim_matches('/').to_string();
        Ok(Self {
            http,
            base_url,
            bucket,
            prefix,
            auth,
        })
    }

    fn object_name(&self, key: &str) -> String {
        if self.prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}/{key}", self.prefix)
        }
    }

    fn encoded(&self, key: &str) -> String {
        utf8_percent_encode(&self.object_name(key), OBJECT_NAME).to_string()
    }

    async fn authorize(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, StorageError> {
        Ok(match &self.auth {
            Some(auth) => request.bearer_auth(auth.access_token().await?),
            None => request,
        })
    }

    async fn send(
        &self,
        request: reqwest::RequestBuilder,
        operation: &'static str,
    ) -> Result<reqwest::Response, StorageError> {
        self.authorize(request)
            .await?
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|e| StorageError::Request(format!("{operation}: {}", redact(&e))))
    }
}

#[async_trait]
impl BlobStore for GcsBlobStore {
    async fn put(&self, key: &str, bytes: Bytes, content_type: &str) -> Result<(), StorageError> {
        let url = format!(
            "{}/upload/storage/v1/b/{}/o?uploadType=media&name={}",
            self.base_url,
            self.bucket,
            self.encoded(key)
        );
        let request = self
            .http
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(bytes);
        let response = self.send(request, "upload").await?;
        if !response.status().is_success() {
            return Err(StorageError::Status {
                status: response.status().as_u16(),
                operation: "upload",
            });
        }
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Bytes>, StorageError> {
        let url = format!(
            "{}/storage/v1/b/{}/o/{}?alt=media",
            self.base_url,
            self.bucket,
            self.encoded(key)
        );
        let response = self.send(self.http.get(url), "download").await?;
        match response.status() {
            StatusCode::NOT_FOUND => Ok(None),
            s if s.is_success() => response
                .bytes()
                .await
                .map(Some)
                .map_err(|e| StorageError::Request(format!("download body: {}", redact(&e)))),
            s => Err(StorageError::Status {
                status: s.as_u16(),
                operation: "download",
            }),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool, StorageError> {
        // Object metadata, not media: a few hundred bytes whatever the size.
        let url = format!(
            "{}/storage/v1/b/{}/o/{}?fields=name",
            self.base_url,
            self.bucket,
            self.encoded(key)
        );
        let response = self.send(self.http.get(url), "stat").await?;
        match response.status() {
            StatusCode::NOT_FOUND => Ok(false),
            s if s.is_success() => Ok(true),
            s => Err(StorageError::Status {
                status: s.as_u16(),
                operation: "stat",
            }),
        }
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let url = format!(
            "{}/storage/v1/b/{}/o/{}",
            self.base_url,
            self.bucket,
            self.encoded(key)
        );
        let response = self.send(self.http.delete(url), "delete").await?;
        match response.status() {
            StatusCode::NOT_FOUND => Ok(()),
            s if s.is_success() => Ok(()),
            s => Err(StorageError::Status {
                status: s.as_u16(),
                operation: "delete",
            }),
        }
    }

    fn backend(&self) -> &'static str {
        "gcs"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_object_names_as_one_segment() {
        let store = GcsBlobStore::new(
            reqwest::Client::new(),
            "b".into(),
            "/env/".into(),
            Some("localhost:4447".into()),
            None,
        )
        .unwrap();
        assert_eq!(store.base_url, "http://localhost:4447");
        assert_eq!(
            store.encoded("blobs/sha256/ab/abc"),
            "env%2Fblobs%2Fsha256%2Fab%2Fabc"
        );
        assert!(store.auth.is_none());
    }
}
