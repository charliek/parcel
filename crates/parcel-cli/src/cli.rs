//! Command-line interface definition.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use parcel_types::{CommentStatus, PublishMode, Visibility};

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
    /// Read, answer and resolve feedback left on a parcel
    #[command(subcommand, after_help = COMMENTS_HELP)]
    Comments(CommentsCommand),
    /// Read and change a parcel's data (JSON its pages keep between visits)
    #[command(subcommand, after_help = DATA_HELP)]
    Data(DataCommand),
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

const COMMENTS_HELP: &str = "\
Comments are left in the web viewer's comment mode, usually pinned to an
element of a page, and numbered per parcel (#1, #2, …; replies take numbers
too). `list` shows which page and element each thread is about.

Seeing and adding comments needs a login: on a private parcel only its owner
can; on a public one, any signed-in user. Only a comment's author can edit
it; its thread's author or the parcel's owner can resolve it; its author or
the owner can delete it.";

const DATA_HELP: &str = "\
Data is JSON a parcel's pages keep between visits (window.parcel.data),
grouped into collections of keyed items and shared by every version.

Anyone who can view the parcel can read its data (public parcels need no
login). Only the parcel's owner can change it: with these commands, or from
the page while the owner is viewing it. Visitors' pages see it read-only.

Collection names and keys are 1–128 letters, digits, '_', '.', ':' or '-',
starting with a letter or digit.";

/// A comment or reply's text: `-` reads it from stdin.
#[derive(Debug, Args)]
pub struct CommentText {
    /// The comment's text, or - to read it from stdin
    #[arg(value_name = "TEXT")]
    pub text: String,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CommentStatusArg {
    /// Unresolved threads
    Open,
    /// Resolved threads
    Resolved,
    /// Every thread
    All,
}

impl From<CommentStatusArg> for CommentStatus {
    fn from(status: CommentStatusArg) -> Self {
        match status {
            CommentStatusArg::Open => CommentStatus::Open,
            CommentStatusArg::Resolved => CommentStatus::Resolved,
            CommentStatusArg::All => CommentStatus::All,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum CommentsCommand {
    /// List comment threads: where on which page, who, and what was asked
    List(CommentsListArgs),
    /// Add a general comment about the whole parcel
    Add(CommentsAddArgs),
    /// Reply to a thread
    Reply(CommentsReplyArgs),
    /// Resolve threads, optionally replying to each first
    Resolve(CommentsResolveArgs),
    /// Reopen resolved threads
    Reopen(CommentNumbersArgs),
    /// Delete a comment (a thread's first comment takes its replies with it)
    Delete(CommentNumberArgs),
}

#[derive(Debug, Args)]
pub struct CommentsListArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// Which threads to show
    #[arg(long, value_enum, default_value = "open")]
    pub status: CommentStatusArg,
    #[command(flatten)]
    pub json: JsonArg,
}

#[derive(Debug, Args)]
pub struct CommentsAddArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    #[command(flatten)]
    pub text: CommentText,
    /// The version the comment is about [default: the URL's version, else the current one]
    #[arg(long, value_name = "N")]
    pub version: Option<i32>,
    #[command(flatten)]
    pub json: JsonArg,
}

#[derive(Debug, Args)]
pub struct CommentsReplyArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// The thread's number
    #[arg(value_name = "NUMBER", value_parser = clap::value_parser!(i32).range(1..))]
    pub number: i32,
    #[command(flatten)]
    pub text: CommentText,
    #[command(flatten)]
    pub json: JsonArg,
}

#[derive(Debug, Args)]
pub struct CommentsResolveArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// Thread numbers
    #[arg(value_name = "NUMBER", required = true,
          value_parser = clap::value_parser!(i32).range(1..))]
    pub numbers: Vec<i32>,
    /// Reply to each thread with this text before resolving it (- reads stdin),
    /// e.g. "Fixed in v4: the bars are blue now"
    #[arg(long, value_name = "TEXT")]
    pub reply: Option<String>,
}

#[derive(Debug, Args)]
pub struct CommentNumbersArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// Thread numbers
    #[arg(value_name = "NUMBER", required = true,
          value_parser = clap::value_parser!(i32).range(1..))]
    pub numbers: Vec<i32>,
}

#[derive(Debug, Args)]
pub struct CommentNumberArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// Comment number
    #[arg(value_name = "NUMBER", value_parser = clap::value_parser!(i32).range(1..))]
    pub number: i32,
}

#[derive(Debug, Subcommand)]
pub enum DataCommand {
    /// List the parcel's collections, or every item in one collection
    List(DataListArgs),
    /// Print one item's value as JSON
    Get(DataGetArgs),
    /// Create or replace an item (owner only)
    Set(DataSetArgs),
    /// Merge a JSON object into an item, creating it if needed (owner only)
    Update(DataUpdateArgs),
    /// Delete one item (owner only)
    Delete(DataItemArgs),
    /// Delete every item in a collection (owner only)
    Clear(DataCollectionArgs),
}

#[derive(Debug, Args)]
pub struct DataListArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// List this collection's items instead of the collections
    #[arg(value_name = "COLLECTION")]
    pub collection: Option<String>,
    /// Print the API's JSON response (a collection's pages merged into one
    /// `{"items": [...]}`)
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DataItemArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// Collection name
    #[arg(value_name = "COLLECTION")]
    pub collection: String,
    /// Item key
    #[arg(value_name = "KEY")]
    pub key: String,
}

#[derive(Debug, Args)]
pub struct DataCollectionArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// Collection name
    #[arg(value_name = "COLLECTION")]
    pub collection: String,
}

#[derive(Debug, Args)]
pub struct DataGetArgs {
    #[command(flatten)]
    pub item: DataItemArgs,
    /// Print the whole item (value, size, updated_at) as the API sent it
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
#[command(after_help = "\
VALUE is inline JSON ('{\"done\": true}', '42', '\"text\"'), @FILE to read a
JSON file, or - to read stdin. With --string it is stored as a JSON string
instead (a single trailing newline from a file or stdin is dropped).")]
pub struct DataSetArgs {
    #[command(flatten)]
    pub item: DataItemArgs,
    /// The value: inline JSON, @FILE, or - for stdin
    #[arg(value_name = "VALUE", allow_negative_numbers = true)]
    pub value: String,
    /// Store the text as a JSON string rather than parsing it as JSON
    #[arg(long)]
    pub string: bool,
    #[command(flatten)]
    pub json: JsonArg,
}

#[derive(Debug, Args)]
#[command(after_help = "\
PATCH is a JSON object merged into the stored value (RFC 7386): its fields
replace the item's, nested objects merge, and a null removes a field. Give
it inline, as @FILE, or - to read stdin.")]
pub struct DataUpdateArgs {
    #[command(flatten)]
    pub item: DataItemArgs,
    /// A JSON object: inline, @FILE, or - for stdin
    #[arg(value_name = "PATCH")]
    pub patch: String,
    #[command(flatten)]
    pub json: JsonArg,
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
    fn parses_comment_and_data_commands() {
        let cli = Cli::try_parse_from([
            "parcel",
            "comments",
            "resolve",
            "abcdefghijkm",
            "1",
            "3",
            "--reply",
            "-",
        ])
        .unwrap();
        let Command::Comments(CommentsCommand::Resolve(args)) = cli.command else {
            panic!("expected comments resolve")
        };
        assert_eq!(args.numbers, vec![1, 3]);
        assert_eq!(args.reply.as_deref(), Some("-"));

        // A negative number is a value, not a flag.
        let cli =
            Cli::try_parse_from(["parcel", "data", "set", "abcdefghijkm", "c", "k", "-5"]).unwrap();
        let Command::Data(DataCommand::Set(args)) = cli.command else {
            panic!("expected data set")
        };
        assert_eq!(args.value, "-5");

        for bad in [
            &["parcel", "comments", "resolve", "abcdefghijkm"][..],
            &["parcel", "comments", "reopen", "abcdefghijkm", "0"],
            &[
                "parcel",
                "comments",
                "list",
                "abcdefghijkm",
                "--status",
                "closed",
            ],
        ] {
            assert_eq!(
                Cli::try_parse_from(bad).unwrap_err().exit_code(),
                2,
                "{bad:?}"
            );
        }
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
