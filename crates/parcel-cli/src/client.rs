//! A thin blocking client for `/api/v1`.
//!
//! Every JSON call returns both the typed value and the raw body, so `--json`
//! can print exactly what the server sent while the human output uses the
//! shared `parcel_types` structs.

use std::cell::OnceCell;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use parcel_types::ServerInfo;
use reqwest::blocking::{multipart::Form, Client, RequestBuilder, Response};
use reqwest::{Certificate, Method, Url};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::config::server_string;
use crate::error::ApiError;

/// Small JSON calls. Uploads and downloads get [`TRANSFER_TIMEOUT`].
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// A decoded JSON response plus the bytes it came from.
pub struct Fetched<T> {
    pub value: T,
    pub raw: Vec<u8>,
}

pub struct Api {
    http: Client,
    base: Url,
    token: Option<String>,
    /// `GET /info`, read at most once per run.
    info: OnceCell<ServerInfo>,
}

impl Api {
    pub fn new(base: &Url, token: Option<&str>) -> Result<Self> {
        // The OS trust store (mkcert, corporate roots) plus Mozilla's, so a
        // container without a CA bundle still reaches a public server.
        let bundled = webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .map(|der| Certificate::from_der(der))
            .collect::<Result<Vec<_>, _>>()
            .context("loading the bundled root certificates")?;
        let http = Client::builder()
            .user_agent(concat!("parcel-cli/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(None)
            .tls_certs_merge(bundled)
            .build()
            .context("building the HTTP client")?;
        Ok(Api {
            http,
            base: base.clone(),
            token: token.map(str::to_string),
            info: OnceCell::new(),
        })
    }

    /// The server's description (limits, features), fetched on first use
    /// and kept for the rest of the run.
    pub fn server_info(&self) -> Result<&ServerInfo> {
        if let Some(info) = self.info.get() {
            return Ok(info);
        }
        let info = self
            .get::<ServerInfo>(self.endpoint(["info"]))
            .context("reading the server's description")?
            .value;
        Ok(self.info.get_or_init(|| info))
    }

    /// Whether the server lists `feature` (one of `parcel_types::FEATURES`).
    pub fn has_feature(&self, feature: &str) -> Result<bool> {
        Ok(self.server_info()?.features.iter().any(|f| f == feature))
    }

    /// Refuse to send a parameter the server doesn't know: an older server
    /// ignores unknown query parameters (`list --tag` would quietly list
    /// everything) and refuses unknown metadata fields with a vaguer error.
    /// `what` completes "this server doesn't support … yet".
    pub fn require_feature(&self, feature: &str, what: &str) -> Result<()> {
        if !self.has_feature(feature)? {
            bail!("this server doesn't support {what} yet (it is older than this CLI)");
        }
        Ok(())
    }

    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    /// `<base>/api/v1/<segments…>`, each segment percent-encoded on its own
    /// (so a published path's `/` separators become real segments only when
    /// the caller splits them).
    pub fn endpoint<'a>(&self, segments: impl IntoIterator<Item = &'a str>) -> Url {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .expect("server URLs are http(s) and always have a path")
            .pop_if_empty()
            .extend(["api", "v1"])
            .extend(segments);
        url
    }

    fn request(&self, method: Method, url: Url) -> RequestBuilder {
        let builder = self.http.request(method, url);
        match &self.token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        }
    }

    /// Send, turning transport failures and non-2xx statuses into errors.
    fn send(&self, builder: RequestBuilder) -> Result<Response> {
        let response = builder.send().map_err(|e| self.transport_error(e))?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.bytes().map(|b| b.to_vec()).unwrap_or_default();
        Err(ApiError::from_response(status.as_u16(), &body).into())
    }

    fn transport_error(&self, err: reqwest::Error) -> anyhow::Error {
        let server = server_string(&self.base);
        if err.is_timeout() {
            anyhow::anyhow!("request to {server} timed out")
        } else if err.is_connect() {
            anyhow::Error::new(err).context(format!("could not connect to {server}"))
        } else {
            anyhow::Error::new(err).context(format!("request to {server} failed"))
        }
    }

    fn decode<T: DeserializeOwned>(&self, response: Response) -> Result<Fetched<T>> {
        let url = response.url().clone();
        let raw = response
            .bytes()
            .map_err(|e| self.transport_error(e))?
            .to_vec();
        let value = serde_json::from_slice(&raw).with_context(|| {
            format!(
                "unexpected response from {} (is this a Parcel server?)",
                url.path()
            )
        })?;
        Ok(Fetched { value, raw })
    }

    pub fn get<T: DeserializeOwned>(&self, url: Url) -> Result<Fetched<T>> {
        let response = self.send(self.request(Method::GET, url).timeout(REQUEST_TIMEOUT))?;
        self.decode(response)
    }

    pub fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        url: Url,
        body: &B,
    ) -> Result<Fetched<T>> {
        let response = self.send(
            self.request(Method::POST, url)
                .json(body)
                .timeout(REQUEST_TIMEOUT),
        )?;
        self.decode(response)
    }

    /// POST with no body (e.g. restore).
    pub fn post_empty<T: DeserializeOwned>(&self, url: Url) -> Result<Fetched<T>> {
        let response = self.send(self.request(Method::POST, url).timeout(REQUEST_TIMEOUT))?;
        self.decode(response)
    }

    pub fn patch<B: Serialize, T: DeserializeOwned>(
        &self,
        url: Url,
        body: &B,
    ) -> Result<Fetched<T>> {
        let response = self.send(
            self.request(Method::PATCH, url)
                .json(body)
                .timeout(REQUEST_TIMEOUT),
        )?;
        self.decode(response)
    }

    pub fn put<B: Serialize, T: DeserializeOwned>(&self, url: Url, body: &B) -> Result<Fetched<T>> {
        let response = self.send(
            self.request(Method::PUT, url)
                .json(body)
                .timeout(REQUEST_TIMEOUT),
        )?;
        self.decode(response)
    }

    /// PUT/PATCH a body that is already JSON text (a data value, kept exactly
    /// as the user wrote it rather than re-serialized with sorted keys).
    pub fn send_json_text<T: DeserializeOwned>(
        &self,
        method: Method,
        url: Url,
        content_type: &'static str,
        body: Vec<u8>,
    ) -> Result<Fetched<T>> {
        let response = self.send(
            self.request(method, url)
                .header(reqwest::header::CONTENT_TYPE, content_type)
                .body(body)
                .timeout(REQUEST_TIMEOUT),
        )?;
        self.decode(response)
    }

    pub fn delete(&self, url: Url) -> Result<()> {
        self.send(self.request(Method::DELETE, url).timeout(REQUEST_TIMEOUT))?;
        Ok(())
    }

    /// A device-flow poll. The body is decoded even on a non-2xx status, so a
    /// server that answers `pending`/`expired` with a 4xx (RFC 8628 style)
    /// works as well as one that answers 200.
    pub fn post_lenient<B: Serialize, T: DeserializeOwned>(&self, url: Url, body: &B) -> Result<T> {
        let response = self
            .request(Method::POST, url)
            .json(body)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .map_err(|e| self.transport_error(e))?;
        let status = response.status();
        let raw = response.bytes().map_err(|e| self.transport_error(e))?;
        match serde_json::from_slice::<T>(&raw) {
            Ok(value) => Ok(value),
            Err(_) if !status.is_success() => {
                Err(ApiError::from_response(status.as_u16(), &raw).into())
            }
            Err(e) => Err(e).context("unexpected response from the device-login endpoint"),
        }
    }

    pub fn upload<T: DeserializeOwned>(&self, url: Url, form: Form) -> Result<Fetched<T>> {
        let response = self.send(
            self.request(Method::POST, url)
                .multipart(form)
                .timeout(TRANSFER_TIMEOUT),
        )?;
        self.decode(response)
    }

    /// GET a raw file; the caller streams the body.
    pub fn download(&self, url: Url) -> Result<Response> {
        self.send(self.request(Method::GET, url).timeout(TRANSFER_TIMEOUT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_encoded_endpoints() {
        let api = Api::new(&Url::parse("http://localhost:8106").unwrap(), None).unwrap();
        let url = api.endpoint(["parcels", "abcdefghijkm"]);
        assert_eq!(
            url.as_str(),
            "http://localhost:8106/api/v1/parcels/abcdefghijkm"
        );

        let path = "assets/my file#1?.js";
        let url = api.endpoint(
            ["parcels", "abcdefghijkm", "versions", "latest", "files"]
                .into_iter()
                .chain(path.split('/')),
        );
        assert_eq!(
            url.as_str(),
            "http://localhost:8106/api/v1/parcels/abcdefghijkm/versions/latest/files/assets/my%20file%231%3F.js"
        );

        let prefixed = Api::new(&Url::parse("https://example.com/parcel").unwrap(), None).unwrap();
        assert_eq!(
            prefixed.endpoint(["me"]).as_str(),
            "https://example.com/parcel/api/v1/me"
        );
    }
}
