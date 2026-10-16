//! Command-line interface definition.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use parcel_types::{PublishMode, Visibility};

use crate::files::parse_file_arg;

const AFTER_HELP: &str = "\
Environment:
  PARCEL_SERVER      Server URL (same as --server)
  PARCEL_PROFILE     Profile name (same as --profile)
  PARCEL_TOKEN       API token; overrides the stored one
  PARCEL_CONFIG_DIR  Directory holding config.toml

Exit codes: 0 ok, 1 error, 2 usage, 3 not logged in / unauthorized, 4 not found.";

/// Publish HTML pages to Parcel and get back a private, sandboxed URL.
#[derive(Debug, Parser)]
#[command(name = "parcel", version, after_help = AFTER_HELP)]
pub struct Cli {
    /// Parcel server URL [default: the profile's server, else https://parcel.stridelabs.ai]
    #[arg(
        long,
        env = "PARCEL_SERVER",
        global = true,
        value_name = "URL",
        help_heading = "Global options"
    )]
    pub server: Option<String>,

    /// Config profile to use
    #[arg(
        long,
        env = "PARCEL_PROFILE",
        global = true,
        value_name = "NAME",
        default_value = "default",
        help_heading = "Global options"
    )]
    pub profile: String,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Sign in (browser device flow, or an existing token)
    Login(LoginArgs),
    /// Revoke and forget this profile's token
    Logout,
    /// Show the signed-in account and storage usage
    Whoami(JsonArg),
    /// Publish a file or folder as a new parcel, or as a new version of one
    Publish(PublishArgs),
    /// List your parcels
    List(ListArgs),
    /// Show a parcel's metadata, versions and files
    Info(InfoArgs),
    /// Write one file of a parcel to stdout (default: the entry page)
    Cat(CatArgs),
    /// Download every file of a parcel version into a directory
    Pull(PullArgs),
    /// Change a parcel's title, description or visibility
    Update(UpdateArgs),
    /// List a parcel's versions
    Versions(RefJsonArgs),
    /// Publish an old version again as the newest one
    Restore(RestoreArgs),
    /// Delete a parcel and all its versions
    Delete(DeleteArgs),
    /// Open a parcel in the browser
    Open(RefArg),
    /// Install or print the bundled agent skill
    #[command(subcommand)]
    Skill(SkillCommand),
    /// Show or change CLI configuration
    #[command(subcommand)]
    Config(ConfigCommand),
}

#[derive(Debug, Args)]
pub struct JsonArg {
    /// Print the API's JSON response
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct RefArg {
    /// Parcel id or viewer URL (…/p/<id>[/v/<n>])
    #[arg(value_name = "REF")]
    pub parcel: String,
}

#[derive(Debug, Args)]
pub struct RefJsonArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    #[command(flatten)]
    pub json: JsonArg,
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    /// Use an existing API token (pcl_…) instead of the browser flow.
    /// Prefer --with-token: command-line arguments are visible to other users
    #[arg(long, value_name = "TOKEN", conflicts_with = "with_token")]
    pub token: Option<String>,
    /// Read an API token from stdin
    #[arg(long)]
    pub with_token: bool,
    /// Print the sign-in URL instead of opening a browser
    #[arg(long)]
    pub no_browser: bool,
}

/// `--public` / `--private`.
#[derive(Debug, Args)]
pub struct VisibilityArgs {
    /// Anyone with the link can view it
    #[arg(long, conflicts_with = "private")]
    pub public: bool,
    /// Only you can view it (the default for new parcels)
    #[arg(long)]
    pub private: bool,
}

impl VisibilityArgs {
    pub fn visibility(&self) -> Option<Visibility> {
        match (self.public, self.private) {
            (true, _) => Some(Visibility::Public),
            (_, true) => Some(Visibility::Private),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ModeArg {
    /// Files sent are added or replaced; files left out are kept
    Merge,
    /// The new version is exactly the files sent
    Replace,
}

impl From<ModeArg> for PublishMode {
    fn from(mode: ModeArg) -> Self {
        match mode {
            ModeArg::Merge => PublishMode::Merge,
            ModeArg::Replace => PublishMode::Replace,
        }
    }
}

#[derive(Debug, Args)]
pub struct PublishArgs {
    /// A file (published under its name and used as the entry page) or a
    /// directory (published recursively; dotfiles and node_modules skipped)
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,

    /// Title (defaults to the page's <title> or first heading)
    #[arg(long)]
    pub title: Option<String>,

    /// Description
    #[arg(long)]
    pub description: Option<String>,

    #[command(flatten)]
    pub visibility: VisibilityArgs,

    /// Entry page path inside the parcel (default: the file itself, index.html,
    /// or the only HTML/Markdown file)
    #[arg(long, value_name = "PATH")]
    pub entry: Option<String>,

    /// Short note recorded with this version ("Draft to legal")
    #[arg(long, value_name = "TEXT")]
    pub label: Option<String>,

    /// Extra file, as PUBLISHED=LOCAL (or LOCAL to keep its file name); repeatable
    #[arg(long = "file", value_name = "PUBLISHED=LOCAL", value_parser = parse_file_arg)]
    pub files: Vec<(String, PathBuf)>,

    /// Publish as a new version of this parcel (id or URL) instead of a new parcel
    #[arg(long = "parcel", visible_alias = "update", value_name = "ID|URL")]
    pub parcel: Option<String>,

    /// How a new version relates to the previous one [default: replace for a
    /// directory, merge for a single file]
    #[arg(long, value_enum, requires = "parcel")]
    pub mode: Option<ModeArg>,

    /// Drop this path from the previous version (merge mode); repeatable
    #[arg(long = "remove", value_name = "PATH", requires = "parcel")]
    pub remove: Vec<String>,

    /// Fail unless the parcel's current version is still N
    #[arg(long, value_name = "N", requires = "parcel")]
    pub base_version: Option<i32>,

    /// Open the parcel in a browser afterwards
    #[arg(long)]
    pub open: bool,

    #[command(flatten)]
    pub json: JsonArg,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Only parcels whose title or description matches
    #[arg(long, short = 'q', value_name = "TEXT")]
    pub query: Option<String>,
    /// Maximum number of parcels to show
    #[arg(long, value_name = "N")]
    pub limit: Option<u32>,
    /// Skip this many parcels (pagination)
    #[arg(long, value_name = "N")]
    pub offset: Option<u32>,
    #[command(flatten)]
    pub json: JsonArg,
}

#[derive(Debug, Args)]
pub struct InfoArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// Show this version's files (also taken from a …/v/<n> URL)
    #[arg(long, value_name = "N")]
    pub version: Option<i32>,
    #[command(flatten)]
    pub json: JsonArg,
}

#[derive(Debug, Args)]
pub struct CatArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// File path inside the parcel [default: the entry page]
    #[arg(value_name = "PATH")]
    pub path: Option<String>,
    /// Version to read [default: the URL's version, else the latest]
    #[arg(long, value_name = "N")]
    pub version: Option<i32>,
}

#[derive(Debug, Args)]
pub struct PullArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// Destination directory [default: ./<id>]
    #[arg(value_name = "DIR")]
    pub dir: Option<PathBuf>,
    /// Version to download [default: the URL's version, else the latest]
    #[arg(long, value_name = "N")]
    pub version: Option<i32>,
    /// Overwrite files that already exist
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// New title
    #[arg(long)]
    pub title: Option<String>,
    /// New description
    #[arg(long)]
    pub description: Option<String>,
    #[command(flatten)]
    pub visibility: VisibilityArgs,
    #[command(flatten)]
    pub json: JsonArg,
}

#[derive(Debug, Args)]
pub struct RestoreArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// Version to restore
    #[arg(long, value_name = "N")]
    pub version: i32,
    #[command(flatten)]
    pub json: JsonArg,
}

#[derive(Debug, Args)]
pub struct DeleteArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// Don't ask for confirmation (required when stdin is not a terminal)
    #[arg(long, short = 'y')]
    pub yes: bool,
}

#[derive(Debug, Subcommand)]
pub enum SkillCommand {
    /// Write the skill to ~/.claude/skills/parcel/ (read by Claude Code, craze and opencode)
    Install(SkillInstallArgs),
    /// Print SKILL.md
    Show,
    /// Print where `parcel skill install` writes SKILL.md by default
    Path,
}

#[derive(Debug, Args)]
pub struct SkillInstallArgs {
    /// Also write ~/.agents/skills/parcel/
    #[arg(long)]
    pub agents: bool,
    /// Write ./.claude/skills/parcel/ (this project) instead of your home directory
    #[arg(long)]
    pub project: bool,
    /// Write SKILL.md and references/ into DIR itself
    #[arg(long, value_name = "DIR", conflicts_with_all = ["project", "agents"])]
    pub dir: Option<PathBuf>,
    /// Overwrite files that differ from the bundled skill
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Show the active profile, server and whether a token is set
    Show,
    /// Set the active profile's server URL
    SetServer {
        #[arg(value_name = "URL")]
        url: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_publish_flags() {
        let cli = Cli::try_parse_from([
            "parcel",
            "publish",
            "site",
            "--update",
            "abcdefghijkm",
            "--file",
            "a.js=b.js",
            "--remove",
            "old.css",
            "--mode",
            "merge",
            "--public",
            "--json",
        ])
        .unwrap();
        let Command::Publish(args) = cli.command else {
            panic!("expected publish")
        };
        assert_eq!(args.parcel.as_deref(), Some("abcdefghijkm"));
        assert_eq!(args.files, vec![("a.js".into(), PathBuf::from("b.js"))]);
        assert_eq!(args.remove, vec!["old.css"]);
        assert!(matches!(args.mode, Some(ModeArg::Merge)));
        assert_eq!(args.visibility.visibility(), Some(Visibility::Public));
        assert!(args.json.json);
    }

    #[test]
    fn update_only_flags_need_a_parcel() {
        let err =
            Cli::try_parse_from(["parcel", "publish", "x.html", "--mode", "replace"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        let err = Cli::try_parse_from(["parcel", "publish", "x.html", "--public", "--private"])
            .unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }
}
