//! `config.toml`: per-profile server URL and API token.
//!
//! ```toml
//! [profiles.default]
//! server = "https://parcel.stridelabs.ai"
//! token = "pcl_…"
//! ```
//!
//! The file holds secrets, so it is written atomically (temp file + rename)
//! with mode 0600 inside a 0700 directory, and — like ssh with a private key —
//! a file that other users can read is refused while it holds a token.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::error;

/// Server used when neither `--server`, `PARCEL_SERVER` nor the profile names one.
pub const DEFAULT_SERVER: &str = "https://parcel.stridelabs.ai";
pub const DEFAULT_PROFILE: &str = "default";
const FILE_NAME: &str = "config.toml";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

impl ConfigFile {
    fn has_token(&self) -> bool {
        self.profiles.values().any(|p| p.token.is_some())
    }
}

/// Where the config lives: `PARCEL_CONFIG_DIR`, else `$XDG_CONFIG_HOME/parcel`,
/// else the OS config directory (`~/.config/parcel`,
/// `~/Library/Application Support/parcel`, `%APPDATA%\parcel`).
pub fn config_dir() -> Result<PathBuf> {
    if let Some(dir) = non_empty_env("PARCEL_CONFIG_DIR") {
        return Ok(PathBuf::from(dir));
    }
    if let Some(xdg) = non_empty_env("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(xdg).join("parcel"));
    }
    let base = directories::BaseDirs::new()
        .context("could not determine a config directory; set PARCEL_CONFIG_DIR")?;
    Ok(base.config_dir().join("parcel"))
}

pub fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Reads and writes one `config.toml`.
#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(dir: &Path) -> Self {
        ConfigStore {
            path: dir.join(FILE_NAME),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the file; a missing file is an empty config.
    pub fn load(&self) -> Result<ConfigFile> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ConfigFile::default()),
            Err(e) => {
                return Err(e).with_context(|| format!("reading {}", self.path.display()));
            }
        };
        let config: ConfigFile =
            toml::from_str(&text).with_context(|| format!("parsing {}", self.path.display()))?;
        if config.has_token() {
            check_private(&self.path)?;
        }
        Ok(config)
    }

    /// Write the whole file atomically with mode 0600.
    pub fn save(&self, config: &ConfigFile) -> Result<()> {
        let dir = self
            .path
            .parent()
            .context("config path has no parent directory")?;
        create_private_dir(dir)?;
        let text = toml::to_string_pretty(config).context("serializing config")?;
        let tmp = dir.join(format!(".{FILE_NAME}.{}.tmp", std::process::id()));
        // A leftover from a crashed run with the same pid would block create_new.
        let _ = fs::remove_file(&tmp);
        let result = (|| -> Result<()> {
            let mut file = open_private(&tmp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&tmp, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result.with_context(|| format!("writing {}", self.path.display()))
    }
}

#[cfg(unix)]
fn check_private(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(path)
        .with_context(|| format!("reading {}", path.display()))?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        bail!(
            "{} holds an API token but is accessible by other users (mode {:o}); \
             run `chmod 600 {}`",
            path.display(),
            mode & 0o777,
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_private(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    if dir.is_dir() {
        return Ok(());
    }
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("creating {}", dir.display()))
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))
}

#[cfg(unix)]
fn open_private(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_private(path: &Path) -> std::io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// Parse and normalize a server URL: http(s) only, no query or fragment, no
/// trailing slash. A path prefix (`https://host/parcel`) is kept.
pub fn parse_server(input: &str) -> Result<Url> {
    let trimmed = input.trim();
    let mut url = Url::parse(trimmed).map_err(|e| {
        error::usage(format!(
            "invalid server URL {trimmed:?}: {e} (expected e.g. {DEFAULT_SERVER})"
        ))
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(error::usage(format!(
            "invalid server URL {trimmed:?}: must be an http:// or https:// URL"
        )));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(error::usage(format!(
            "invalid server URL {trimmed:?}: must not have a query or fragment"
        )));
    }
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(&path);
    Ok(url)
}

/// The display/storage form of a normalized server URL (no trailing slash).
pub fn server_string(url: &Url) -> String {
    url.as_str().trim_end_matches('/').to_string()
}

fn same_server(a: &Url, b: &str) -> bool {
    parse_server(b).is_ok_and(|b| server_string(a) == server_string(&b))
}

pub fn validate_profile_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(error::usage(format!(
            "invalid profile name {name:?}: use letters, digits, '-' or '_'"
        )))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerSource {
    /// `--server` or `PARCEL_SERVER`.
    Flag,
    Profile,
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    Env,
    Profile,
    None,
}

/// The effective server and credentials for one invocation.
#[derive(Debug)]
pub struct Settings {
    pub profile: String,
    pub server: Url,
    pub server_source: ServerSource,
    pub token: Option<String>,
    pub token_source: TokenSource,
    /// The profile's server when it holds a token for a different server than
    /// the one in use (that token is never sent elsewhere).
    pub token_server_mismatch: Option<String>,
    pub store: ConfigStore,
    pub config: ConfigFile,
}

impl Settings {
    /// Resolve settings. Precedence: `--server`/`PARCEL_SERVER` > profile >
    /// default for the server; `PARCEL_TOKEN` > profile for the token. A
    /// stored token is bound to the server it was issued by and is only used
    /// when that is the server in effect, so `--server https://elsewhere`
    /// never leaks it.
    pub fn resolve(
        flag_server: Option<&str>,
        profile: &str,
        env_token: Option<String>,
        store: ConfigStore,
    ) -> Result<Self> {
        validate_profile_name(profile)?;
        let config = store.load()?;
        let stored = config.profiles.get(profile).cloned().unwrap_or_default();
        let stored_server = stored
            .server
            .clone()
            .unwrap_or_else(|| DEFAULT_SERVER.to_string());

        let (server, server_source) = match flag_server.filter(|s| !s.trim().is_empty()) {
            Some(s) => (parse_server(s)?, ServerSource::Flag),
            None => match &stored.server {
                Some(s) => (
                    parse_server(s).with_context(|| {
                        format!("profile {profile:?} in {}", store.path().display())
                    })?,
                    ServerSource::Profile,
                ),
                None => (parse_server(DEFAULT_SERVER)?, ServerSource::Default),
            },
        };

        let mut token_server_mismatch = None;
        let (token, token_source) = match env_token.filter(|t| !t.trim().is_empty()) {
            Some(t) => (Some(t.trim().to_string()), TokenSource::Env),
            None => match stored.token {
                Some(t) if same_server(&server, &stored_server) => (Some(t), TokenSource::Profile),
                Some(_) => {
                    token_server_mismatch = Some(stored_server);
                    (None, TokenSource::None)
                }
                None => (None, TokenSource::None),
            },
        };

        Ok(Settings {
            profile: profile.to_string(),
            server,
            server_source,
            token,
            token_source,
            token_server_mismatch,
            store,
            config,
        })
    }

    pub fn server_display(&self) -> String {
        server_string(&self.server)
    }

    /// The token, or an exit-3 error explaining how to log in.
    pub fn require_token(&self) -> Result<&str> {
        if let Some(token) = &self.token {
            return Ok(token);
        }
        let mut message = format!("not logged in to {}", self.server_display());
        if let Some(other) = &self.token_server_mismatch {
            message.push_str(&format!(
                " (profile {:?} is logged in to {other})",
                self.profile
            ));
        }
        message.push_str("; run `parcel login`");
        if self.profile != DEFAULT_PROFILE {
            message.push_str(&format!(" --profile {}", self.profile));
        }
        Err(error::auth(message))
    }

    pub fn profile_entry(&self) -> Profile {
        self.config
            .profiles
            .get(&self.profile)
            .cloned()
            .unwrap_or_default()
    }

    /// Replace this profile's entry and save the file.
    pub fn save_profile(&mut self, profile: Profile) -> Result<()> {
        if profile == Profile::default() {
            self.config.profiles.remove(&self.profile);
        } else {
            self.config.profiles.insert(self.profile.clone(), profile);
        }
        self.store.save(&self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(&dir.path().join("nested"));
        (dir, store)
    }

    fn sample() -> ConfigFile {
        let mut config = ConfigFile::default();
        config.profiles.insert(
            "default".into(),
            Profile {
                server: Some("http://localhost:8106".into()),
                token: Some("pcl_secret".into()),
            },
        );
        config.profiles.insert(
            "staging".into(),
            Profile {
                server: Some("https://parcel-staging.stridelabs.ai".into()),
                token: None,
            },
        );
        config
    }

    #[test]
    fn round_trips_and_writes_private_files() {
        let (_dir, store) = store();
        assert_eq!(store.load().unwrap(), ConfigFile::default());
        store.save(&sample()).unwrap();
        assert_eq!(store.load().unwrap(), sample());

        let text = fs::read_to_string(store.path()).unwrap();
        assert!(text.contains("[profiles.default]"), "{text}");
        assert!(
            text.contains("server = \"http://localhost:8106\""),
            "{text}"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(store.path()).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            let dir_mode = fs::metadata(store.path().parent().unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(dir_mode & 0o777, 0o700);
        }
        // No temp files left behind.
        let leftovers: Vec<_> = fs::read_dir(store.path().parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, vec![std::ffi::OsString::from("config.toml")]);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_readable_token_file() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, store) = store();
        store.save(&sample()).unwrap();
        fs::set_permissions(store.path(), fs::Permissions::from_mode(0o644)).unwrap();
        let err = store.load().unwrap_err().to_string();
        assert!(err.contains("accessible by other users"), "{err}");
        assert!(err.contains("chmod 600"), "{err}");

        // Without a token there is nothing to protect.
        let mut no_token = sample();
        no_token.profiles.get_mut("default").unwrap().token = None;
        fs::set_permissions(store.path(), fs::Permissions::from_mode(0o600)).unwrap();
        store.save(&no_token).unwrap();
        fs::set_permissions(store.path(), fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(store.load().unwrap(), no_token);
    }

    #[test]
    fn normalizes_server_urls() {
        let url = parse_server(" https://Parcel.Example.com/ ").unwrap();
        assert_eq!(server_string(&url), "https://parcel.example.com");
        let url = parse_server("http://localhost:8106/prefix/").unwrap();
        assert_eq!(server_string(&url), "http://localhost:8106/prefix");
        assert!(parse_server("ftp://x").is_err());
        assert!(parse_server("parcel.stridelabs.ai").is_err());
        assert!(parse_server("https://x/?a=b").is_err());
    }

    #[test]
    fn resolves_server_and_token_precedence() {
        let (_dir, store) = store();
        store.save(&sample()).unwrap();

        // Profile server and token.
        let s = Settings::resolve(None, "default", None, store.clone()).unwrap();
        assert_eq!(s.server_display(), "http://localhost:8106");
        assert_eq!(s.server_source, ServerSource::Profile);
        assert_eq!(s.token.as_deref(), Some("pcl_secret"));
        assert_eq!(s.token_source, TokenSource::Profile);

        // Same server via the flag (trailing slash ignored) keeps the token.
        let s = Settings::resolve(
            Some("http://localhost:8106/"),
            "default",
            None,
            store.clone(),
        )
        .unwrap();
        assert_eq!(s.token_source, TokenSource::Profile);

        // A different server never receives the stored token.
        let s = Settings::resolve(Some("https://evil.example"), "default", None, store.clone())
            .unwrap();
        assert_eq!(s.token, None);
        assert_eq!(
            s.token_server_mismatch.as_deref(),
            Some("http://localhost:8106")
        );
        let err = s.require_token().unwrap_err();
        assert_eq!(error::exit_code(&err), error::EXIT_AUTH);
        assert!(err
            .to_string()
            .contains("logged in to http://localhost:8106"));

        // PARCEL_TOKEN wins everywhere.
        let s = Settings::resolve(
            Some("https://evil.example"),
            "default",
            Some("pcl_env".into()),
            store.clone(),
        )
        .unwrap();
        assert_eq!(s.token.as_deref(), Some("pcl_env"));
        assert_eq!(s.token_source, TokenSource::Env);

        // A profile without a server uses the default.
        let s = Settings::resolve(None, "fresh", None, store.clone()).unwrap();
        assert_eq!(s.server_display(), DEFAULT_SERVER);
        assert_eq!(s.server_source, ServerSource::Default);
        let err = s.require_token().unwrap_err().to_string();
        assert!(err.contains("--profile fresh"), "{err}");

        assert!(Settings::resolve(None, "bad name", None, store).is_err());
    }
}
