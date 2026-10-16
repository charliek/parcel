//! Command implementations. Results go to stdout; progress, prompts and
//! warnings to stderr; errors are returned and printed once by `main`.

mod auth;
mod comments;
mod data;
mod local;
mod manage;
mod publish;
mod read;

use std::io::Read;

use anyhow::{Context, Result};
use parcel_types::ids::parse_parcel_ref;

use crate::cli::{Cli, Command};
use crate::client::Api;
use crate::config::{self, ConfigStore, Settings};
use crate::error;

pub fn run(cli: Cli) -> Result<()> {
    let Cli {
        server,
        profile,
        command,
    } = cli;
    match command {
        // Never touches the config or the network, so a broken config.toml
        // can't stop an agent from installing the skill.
        Command::Skill(cmd) => local::skill(cmd),
        command => {
            let settings = load_settings(server.as_deref(), &profile)?;
            dispatch(settings, command)
        }
    }
}

fn load_settings(server: Option<&str>, profile: &str) -> Result<Settings> {
    let store = ConfigStore::new(&config::config_dir()?);
    Settings::resolve(
        server,
        profile,
        config::non_empty_env("PARCEL_TOKEN"),
        store,
    )
}

fn dispatch(settings: Settings, command: Command) -> Result<()> {
    match command {
        Command::Login(args) => auth::login(settings, args),
        Command::Logout => auth::logout(settings),
        Command::Whoami(args) => auth::whoami(&settings, args.json),
        Command::Publish(args) => publish::publish(&settings, args),
        Command::List(args) => read::list(&settings, args),
        Command::Info(args) => read::info(&settings, args),
        Command::Cat(args) => read::cat(&settings, args),
        Command::Pull(args) => read::pull(&settings, args),
        Command::Versions(args) => read::versions(&settings, &args.parcel, args.json.json),
        Command::Open(args) => read::open(&settings, &args),
        Command::Update(args) => manage::update(&settings, args),
        Command::Restore(args) => manage::restore(&settings, args),
        Command::Delete(args) => manage::delete(&settings, args),
        Command::Comments(cmd) => comments::run(&settings, cmd),
        Command::Data(cmd) => data::run(&settings, cmd),
        Command::Config(cmd) => local::config(settings, cmd),
        Command::Skill(_) => unreachable!("handled before loading settings"),
    }
}

/// A client that sends the token when there is one (reads of public parcels
/// work signed out).
fn api(settings: &Settings) -> Result<Api> {
    Api::new(&settings.server, settings.token.as_deref())
}

/// A client for calls that need a signed-in user; exit 3 without a token.
fn authed_api(settings: &Settings) -> Result<Api> {
    let token = settings.require_token()?;
    Api::new(&settings.server, Some(token))
}

/// A parcel id or viewer URL → (id, version pinned by the URL).
pub fn parse_ref(input: &str) -> Result<(String, Option<i32>)> {
    parse_parcel_ref(input).ok_or_else(|| {
        error::usage(format!(
            "{input:?} is not a parcel id or URL (expected a 12-character id or …/p/<id>)"
        ))
    })
}

/// The `{version}` path segment: a number, or `latest`.
fn version_segment(version: Option<i32>) -> String {
    version.map_or_else(|| "latest".to_string(), |v| v.to_string())
}

/// All of stdin as text, for arguments given as `-`.
fn read_stdin(what: &str) -> Result<String> {
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .with_context(|| format!("reading {what} from stdin"))?;
    Ok(text)
}

/// Try to open a URL in the default browser without waiting for it.
fn open_in_browser(url: &str) {
    if let Err(e) = open::that_detached(url) {
        eprintln!("warning: could not open a browser ({e}); open the URL above yourself");
    }
}

/// Whether to launch a browser for interactive flows: not over SSH and only
/// when a person is plausibly watching stdout.
fn can_launch_browser() -> bool {
    use std::io::IsTerminal;
    std::env::var_os("SSH_CONNECTION").is_none() && std::io::stdout().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_refs_for_commands() {
        assert_eq!(
            parse_ref("abcdefghijkm").unwrap(),
            ("abcdefghijkm".to_string(), None)
        );
        assert_eq!(
            parse_ref("http://localhost:5178/p/abcdefghijkm/v/7").unwrap(),
            ("abcdefghijkm".to_string(), Some(7))
        );
        let err = parse_ref("not-an-id").unwrap_err();
        assert_eq!(error::exit_code(&err), error::EXIT_USAGE);
        assert!(err.to_string().contains("not a parcel id or URL"));
    }

    #[test]
    fn version_segments() {
        assert_eq!(version_segment(None), "latest");
        assert_eq!(version_segment(Some(3)), "3");
    }
}
