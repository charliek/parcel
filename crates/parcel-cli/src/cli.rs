//! Command-line interface definition.

use std::path::PathBuf;

use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};
use parcel_types::{CommentStatus, PublishMode, ShareRole, Visibility};

use crate::files::parse_file_arg;

/// What a REF argument accepts (`parcel_types::ids::ParcelRef`).
const REF_HELP: &str = "Parcel id, viewer URL (…/p/<id>[/v/<n>] or …/@<handle>/<slug>), \
                        @<handle>/<slug>, or one of your slugs";

const AFTER_HELP: &str = "\
Environment:
  PARCEL_SERVER      Server URL (same as --server)
  PARCEL_PROFILE     Profile name (same as --profile)
  PARCEL_TOKEN       API token; overrides the stored one
  PARCEL_CONFIG_DIR  Directory holding config.toml
  PARCEL_PROJECT     Project for new parcels' project: tag (as publish --project)

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
    /// List and search your parcels, the ones shared with you, or both
    List(ListArgs),
    /// Show a parcel's metadata, versions and files
    Info(InfoArgs),
    /// Write one file of a parcel to stdout (default: the entry page)
    Cat(CatArgs),
    /// Download every file of a parcel version into a directory
    Pull(PullArgs),
    /// Change a parcel's title, description, visibility, tags, slug or archive state
    Update(UpdateArgs),
    /// Add or remove a parcel's tags
    Tag(TagArgs),
    /// List the tags on your parcels with counts, or rename or delete one everywhere
    Tags(TagsArgs),
    /// Hide a parcel from lists and search (it stays viewable by URL)
    Archive(RefJsonArgs),
    /// Bring an archived parcel back into lists and search
    Unarchive(RefJsonArgs),
    /// Print a parcel's link (its clean URL when it has a slug), or a Markdown link
    Link(LinkArgs),
    /// Share a parcel with people by email
    #[command(after_help = SHARING_HELP)]
    Share(ShareArgs),
    /// Stop sharing a parcel with people
    #[command(after_help = SHARING_HELP)]
    Unshare(UnshareArgs),
    /// List the people a parcel is shared with
    #[command(after_help = SHARING_HELP)]
    Shares(RefJsonArgs),
    /// List a parcel's versions
    Versions(RefJsonArgs),
    /// Publish an old version again as the newest one
    Restore(RestoreArgs),
    /// Delete a parcel and all its versions
    Delete(DeleteArgs),
    /// Open a parcel in the browser
    Open(OpenArgs),
    /// Show or change your handle (the @<handle> in your parcels' clean URLs)
    Handle(HandleArgs),
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
    #[arg(value_name = "REF", help = REF_HELP)]
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

/// `--private` / `--internal` / `--public`.
#[derive(Debug, Args)]
#[group(multiple = false)]
pub struct VisibilityArgs {
    /// Only you and the people you share it with (the default for new parcels)
    #[arg(long)]
    pub private: bool,
    /// Anyone signed in to this Parcel server can view and comment
    #[arg(long)]
    pub internal: bool,
    /// Anyone with the link can view it, signed in or not (servers can turn
    /// this off)
    #[arg(long)]
    pub public: bool,
}

impl VisibilityArgs {
    pub fn visibility(&self) -> Option<Visibility> {
        if self.public {
            Some(Visibility::Public)
        } else if self.internal {
            Some(Visibility::Internal)
        } else if self.private {
            Some(Visibility::Private)
        } else {
            None
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
#[command(group(ArgGroup::new("target").args(["parcel", "slug"])))]
#[command(after_help = PUBLISH_HELP)]
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

    /// Publish as a new version of this parcel (id, URL or slug) instead of a new parcel
    #[arg(long = "parcel", visible_alias = "update", value_name = "REF")]
    pub parcel: Option<String>,

    /// Publish under this slug (e.g. <repo>/<plan-name>): a new parcel the
    /// first time, a new version of the same parcel (every file replaced)
    /// after that, from any machine
    #[arg(long, value_name = "SLUG", conflicts_with_all = ["parcel", "mode", "remove"])]
    pub slug: Option<String>,

    /// Add this tag (repeatable); on a new version, added to the parcel's tags
    #[arg(long = "tag", value_name = "TAG")]
    pub tags: Vec<String>,

    /// What the page is (= --tag kind:KIND): plan, discovery, research,
    /// report, review, dashboard, prototype, notes, …
    #[arg(long, value_name = "KIND")]
    pub kind: Option<String>,

    /// Name for a new parcel's project:<name> tag [default: the current
    /// directory's git repository]
    ///
    /// PARCEL_PROJECT is read by `publish` itself, not by clap: only the
    /// flag is an explicit request an older server must refuse.
    #[arg(long, value_name = "NAME")]
    pub project: Option<String>,

    /// Don't add the automatic project:<name> tag to a new parcel
    #[arg(long)]
    pub no_auto_tags: bool,

    /// Don't record where this was published from (repository, branch,
    /// commit, machine)
    #[arg(long)]
    pub no_source: bool,

    /// How a new version relates to the previous one [default: replace for a
    /// directory, merge for a single file]
    #[arg(long, value_enum, requires = "parcel")]
    pub mode: Option<ModeArg>,

    /// Drop this path from the previous version (merge mode); repeatable
    #[arg(long = "remove", value_name = "PATH", requires = "parcel")]
    pub remove: Vec<String>,

    /// Fail unless the parcel's current version is still N (with --slug:
    /// also fail when no parcel has the slug yet)
    #[arg(long, value_name = "N", requires = "target")]
    pub base_version: Option<i32>,

    /// Open the parcel in a browser afterwards
    #[arg(long)]
    pub open: bool,

    #[command(flatten)]
    pub json: JsonArg,
}

const PUBLISH_HELP: &str = "\
Run inside a git repository, a new parcel gets the tag project:<repo> (the
origin remote's name, else the main checkout's folder), and every version
records the repository, branch, commit and machine it came from, which only
you see. --project / PARCEL_PROJECT names the project instead;
--no-auto-tags and --no-source turn either off.";

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Search titles, descriptions, tags, slugs, ids and page text; a snippet of
    /// the matching text shows under each row
    #[arg(long, short = 'q', value_name = "TEXT")]
    pub query: Option<String>,
    /// Only parcels with this tag (repeatable: all of them)
    #[arg(long = "tag", value_name = "TAG")]
    pub tags: Vec<String>,
    /// Only this project's parcels (= --tag project:NAME)
    #[arg(long, value_name = "NAME")]
    pub project: Option<String>,
    /// Only parcels of this kind (= --tag kind:KIND)
    #[arg(long, value_name = "KIND")]
    pub kind: Option<String>,
    /// Parcels other people shared with you by email, instead of yours
    #[arg(long, conflicts_with = "all")]
    pub shared: bool,
    /// Yours and the ones shared with you
    #[arg(long)]
    pub all: bool,
    /// Include archived parcels
    #[arg(long, conflicts_with = "archived_only")]
    pub archived: bool,
    /// Only archived parcels
    #[arg(long)]
    pub archived_only: bool,
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
    /// Show this version's files (also taken from a …/p/<id>/v/<n> or clean-URL ?v=<n> link)
    #[arg(long, value_name = "N")]
    pub version: Option<i32>,
    #[command(flatten)]
    pub json: JsonArg,
}

#[derive(Debug, Args)]
pub struct LinkArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// Link to this page of the parcel (a published .html or .md path)
    #[arg(long, value_name = "PATH")]
    pub page: Option<String>,
    /// Print a Markdown link, [title](url)
    #[arg(long)]
    pub markdown: bool,
    /// Print {url, canonical_url, markdown} as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct OpenArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// Open this page of the parcel (a published .html or .md path)
    #[arg(long, value_name = "PATH")]
    pub page: Option<String>,
    /// Print {url} as JSON (the browser still opens)
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct HandleArgs {
    /// Change it to this (your old handle keeps working in links, and no
    /// one else can take it)
    #[arg(value_name = "NEW")]
    pub handle: Option<String>,
    /// Print {handle, previous} as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct TagArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// Tags to add
    #[arg(long, value_name = "TAG", num_args = 1.., required_unless_present = "remove")]
    pub add: Vec<String>,
    /// Tags to remove
    #[arg(long, value_name = "TAG", num_args = 1..)]
    pub remove: Vec<String>,
    #[command(flatten)]
    pub json: JsonArg,
}

#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct TagsArgs {
    #[command(subcommand)]
    pub command: Option<TagsCommand>,
    #[command(flatten)]
    pub list: TagsListArgs,
}

#[derive(Debug, Args)]
pub struct TagsListArgs {
    /// Only tags starting with this (e.g. project:)
    #[arg(long, value_name = "TEXT")]
    pub prefix: Option<String>,
    /// Count the parcels shared with you instead of yours
    #[arg(long, conflicts_with = "all")]
    pub shared: bool,
    /// Count yours and the ones shared with you
    #[arg(long)]
    pub all: bool,
    /// Print {tags: [{tag, count}], total} as JSON (every page)
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum TagsCommand {
    /// Rename a tag on every parcel you own (into an existing tag: the two merge)
    Rename(TagsRenameArgs),
    /// Remove a tag from every parcel you own
    Delete(TagsDeleteArgs),
}

#[derive(Debug, Args)]
pub struct TagsRenameArgs {
    #[arg(value_name = "FROM")]
    pub from: String,
    #[arg(value_name = "TO")]
    pub to: String,
    /// Print {updated} as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct TagsDeleteArgs {
    #[arg(value_name = "TAG")]
    pub tag: String,
    /// Print {updated} as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct CatArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// File path inside the parcel [default: the URL's ?page=, else the entry page]
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
    /// Add a tag (repeatable)
    #[arg(long = "add-tag", value_name = "TAG", conflicts_with = "set_tags")]
    pub add_tags: Vec<String>,
    /// Remove a tag (repeatable)
    #[arg(long = "remove-tag", value_name = "TAG", conflicts_with = "set_tags")]
    pub remove_tags: Vec<String>,
    /// Replace every tag with these, comma-separated ("" removes them all)
    #[arg(long, value_name = "TAGS")]
    pub set_tags: Option<String>,
    /// Give the parcel this slug ("" removes it); the old one keeps
    /// redirecting here
    #[arg(long, value_name = "SLUG")]
    pub slug: Option<String>,
    /// Hide it from lists and search (it stays viewable by URL)
    #[arg(long, conflicts_with = "unarchive")]
    pub archive: bool,
    /// Bring it back into lists and search
    #[arg(long)]
    pub unarchive: bool,
    #[command(flatten)]
    pub json: JsonArg,
}

const SHARING_HELP: &str = "\
Sharing is by email. Someone you share with gets access the next time they
open the parcel signed in with that address, including their first sign-in,
so you can share before they have an account. Only the owner shares.

Roles: a viewer sees the parcel, its comments and its data; a commenter can
also comment. Only you change the parcel or its data.

Sharing matters for private parcels: on an internal or public one everyone
signed in can already view and comment. Unsharing cuts off pages the person
has open at once.";

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RoleArg {
    /// See the parcel, its comments and its data
    Viewer,
    /// Also comment
    Commenter,
}

impl From<RoleArg> for ShareRole {
    fn from(role: RoleArg) -> Self {
        match role {
            RoleArg::Viewer => ShareRole::Viewer,
            RoleArg::Commenter => ShareRole::Commenter,
        }
    }
}

#[derive(Debug, Args)]
pub struct ShareArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// Their email addresses
    #[arg(value_name = "EMAIL", required = true)]
    pub emails: Vec<String>,
    /// What they may do (re-sharing with someone changes their role)
    #[arg(long, value_enum, default_value = "viewer")]
    pub role: RoleArg,
    /// Print the API's JSON for the parcel's shares afterwards
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct UnshareArgs {
    #[command(flatten)]
    pub parcel: RefArg,
    /// Their email addresses
    #[arg(value_name = "EMAIL", required = true)]
    pub emails: Vec<String>,
    /// Print the API's JSON for the parcel's shares afterwards
    #[arg(long)]
    pub json: bool,
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

Seeing and adding comments needs a login. On a private parcel, its owner and
the people it is shared with see them (viewers read, commenters also add);
on an internal or public one, anyone signed in. Only a comment's author can
edit it; its thread's author or the parcel's owner can resolve it; its
author or the owner can delete it.";

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

    #[test]
    fn visibility_flags_are_exclusive() {
        let cli = Cli::try_parse_from(["parcel", "update", "abcdefghijkm", "--internal"]).unwrap();
        let Command::Update(args) = cli.command else {
            panic!("expected update")
        };
        assert_eq!(args.visibility.visibility(), Some(Visibility::Internal));
        for pair in [
            ["--internal", "--public"],
            ["--internal", "--private"],
            ["--private", "--public"],
        ] {
            let err =
                Cli::try_parse_from(["parcel", "publish", "x.html", pair[0], pair[1]]).unwrap_err();
            assert_eq!(err.exit_code(), 2, "{pair:?}");
        }
    }

    #[test]
    fn parses_sharing_commands() {
        let cli = Cli::try_parse_from([
            "parcel",
            "share",
            "abcdefghijkm",
            "ada@example.com",
            "bob@example.com",
            "--role",
            "commenter",
        ])
        .unwrap();
        let Command::Share(args) = cli.command else {
            panic!("expected share")
        };
        assert_eq!(args.emails, vec!["ada@example.com", "bob@example.com"]);
        assert_eq!(ShareRole::from(args.role), ShareRole::Commenter);

        let cli = Cli::try_parse_from(["parcel", "share", "abcdefghijkm", "a@x.com"]).unwrap();
        let Command::Share(args) = cli.command else {
            panic!("expected share")
        };
        assert_eq!(ShareRole::from(args.role), ShareRole::Viewer);

        for bad in [
            &["parcel", "share", "abcdefghijkm"][..],
            &["parcel", "unshare", "abcdefghijkm"],
            &[
                "parcel",
                "share",
                "abcdefghijkm",
                "a@x.com",
                "--role",
                "editor",
            ],
            &["parcel", "list", "--shared", "--all"],
        ] {
            assert_eq!(
                Cli::try_parse_from(bad).unwrap_err().exit_code(),
                2,
                "{bad:?}"
            );
        }
    }
}
