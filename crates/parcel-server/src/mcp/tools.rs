//! The tools. Each one resolves its caller, then calls the function the
//! matching REST handler calls, and returns that handler's JSON shape.

use base64::Engine as _;
use bytes::Bytes;
use parcel_types::{
    CreateParcelMetadata, NewVersionMetadata, PublishMode, ShareRole, UpdateParcelRequest,
    Visibility,
};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router, ErrorData, RoleServer, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use stridelabs_mcp::{clamp_limit, clamp_offset, Page, ToolCall, ToolError};

use super::{caller, tool_error, PUBLISH_TIMEOUT, SCOPE_READ, SCOPE_WRITE};
use crate::http::parcels::{self as parcel_api, ListQuery};
use crate::http::sharing::{self as sharing_api, SharedQuery};
use crate::http::versions as version_api;
use crate::publish;
use crate::state::AppState;

/// Largest file `read_file` returns; bigger ones are for the CLI.
const MAX_READ_BYTES: i64 = 1024 * 1024;

const INSTRUCTIONS: &str = "\
Parcel hosts HTML pages (\"parcels\") at private links that render in a sandboxed browser view: \
reports, dashboards, charts, prototypes, diagrams, long documents. Publish when the result is \
something to look at or read as a page; answer in chat when it fits there.

Publishing: send one self-contained index.html (CSS in <style>, JS in <script>, images inline \
or as data: URIs) or several files with relative paths (assets/app.js, never /assets/app.js). \
Give every page a <title>, the viewport meta, light and dark themes, and phone-width layout. A \
.md entry page is rendered too. Parcels are private by default: only the owner, and people it \
is shared with by email, can open them. Always give the user the parcel's `url`.

Revising: update_parcel publishes a new version to the same URL (merge keeps unsent files, \
replace sends the whole set). Pass base_version when the user may be editing too.

The sandbox: pages have an opaque origin on a separate domain. localStorage, sessionStorage \
and cookies live in memory and reset on reload; IndexedDB throws. Relative fetches of the \
parcel's own files work; external links need target=\"_blank\". For data that must survive a \
reload, the page can use window.parcel.data (see https://github.com/charliek/parcel).";

#[derive(Clone)]
pub struct ParcelMcp {
    state: AppState,
}

impl ParcelMcp {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

/// A file to publish.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FileInput {
    /// Published path, relative: `index.html`, `assets/app.js`, `data/q3.json`.
    pub path: String,
    /// The file's contents: text, or base64 when `encoding` is `base64`.
    pub content: String,
    /// `utf8` (the default) for text, `base64` for binary files such as images and fonts.
    #[serde(default)]
    pub encoding: Encoding,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    #[default]
    Utf8,
    Base64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListParams {
    /// `mine` (the default): parcels you own. `shared`: parcels other people shared with you.
    #[serde(default)]
    pub which: Which,
    /// Case-insensitive match on title, description or id (your own parcels only).
    pub query: Option<String>,
    /// Page size, 1–50 (default 20).
    pub limit: Option<i64>,
    /// Items to skip (default 0).
    pub offset: Option<i64>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Which {
    #[default]
    Mine,
    Shared,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ParcelRef {
    /// Parcel id (12 characters) or its URL (…/p/<id>).
    pub parcel: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadFileParams {
    /// Parcel id or URL; a URL ending in /v/<n> reads that version.
    pub parcel: String,
    /// The file's published path (default: the version's entry page).
    pub path: Option<String>,
    /// Version number (default: the current version, or the one in the URL).
    pub version: Option<i32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PublishParams {
    /// The files. The entry page is `entry`, else the only HTML/Markdown page, else index.html.
    pub files: Vec<FileInput>,
    /// Title (default: the entry page's <title> or first heading).
    pub title: Option<String>,
    /// A longer description.
    pub description: Option<String>,
    /// `private` (default; only you and people you share with), `internal` (everyone signed in
    /// to this Parcel) or `public` (anyone with the link, if this Parcel allows it).
    pub visibility: Option<Visibility>,
    /// The entry page's path, when it isn't obvious.
    pub entry: Option<String>,
    /// A note about this version.
    pub label: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateParams {
    /// Parcel id or URL. Only its owner can update it.
    pub parcel: String,
    /// Files for a new version: added or replaced (merge) or the whole set (replace).
    #[serde(default)]
    pub files: Vec<FileInput>,
    /// Paths to drop in the new version (merge mode only).
    #[serde(default)]
    pub remove: Vec<String>,
    /// `merge` (default): unsent files are kept. `replace`: the version is exactly `files`.
    pub mode: Option<PublishMode>,
    /// A new entry page path.
    pub entry: Option<String>,
    /// A note about this version.
    pub label: Option<String>,
    /// Fail if the current version is no longer this one (someone else published).
    pub base_version: Option<i32>,
    /// New title.
    pub title: Option<String>,
    /// New description.
    pub description: Option<String>,
    /// New visibility: `private`, `internal` or `public`.
    pub visibility: Option<Visibility>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ShareParams {
    /// Parcel id or URL. Only its owner can share it.
    pub parcel: String,
    /// Their email address; they get access when they sign in with it.
    pub email: String,
    /// `viewer` (default) or `commenter`.
    pub role: Option<ShareRole>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UnshareParams {
    /// Parcel id or URL.
    pub parcel: String,
    /// The email it is shared with.
    pub email: String,
}

#[derive(Debug, Serialize)]
struct FileInfo {
    path: String,
    version: i32,
    content_type: String,
    size_bytes: usize,
    encoding: Encoding,
}

#[derive(Debug, Serialize)]
struct WhoAmI {
    email: String,
    name: String,
    parcel_url: String,
    scopes: Vec<String>,
}

fn structured<T: Serialize>(value: &T) -> Result<CallToolResult, ToolError> {
    Ok(CallToolResult::structured(
        serde_json::to_value(value).map_err(ToolError::internal)?,
    ))
}

/// The id (and any version) in a parcel id or URL.
fn parcel_ref(input: &str) -> Result<(String, Option<i32>), ToolError> {
    parcel_types::ids::parse_parcel_ref(input.trim()).ok_or_else(|| {
        ToolError::user(format!(
            "{input:?} is not a parcel id or URL (…/p/<12-character id>)"
        ))
    })
}

fn decode_files(files: Vec<FileInput>) -> Result<Vec<(String, Bytes)>, ToolError> {
    files
        .into_iter()
        .map(|f| {
            let bytes = match f.encoding {
                Encoding::Utf8 => Bytes::from(f.content),
                Encoding::Base64 => base64::engine::general_purpose::STANDARD
                    .decode(f.content.trim())
                    .map(Bytes::from)
                    .map_err(|_| {
                        ToolError::user(format!("{:?}: content is not valid base64", f.path))
                    })?,
            };
            Ok((f.path, bytes))
        })
        .collect()
}

#[tool_router]
impl ParcelMcp {
    #[tool(
        description = "Who you are signed in to Parcel as, and the scopes this connection grants.",
        annotations(read_only_hint = true)
    )]
    async fn whoami(&self, ctx: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        ToolCall::new("whoami")
            .run(&ctx, |principal| async move {
                let me = caller(&self.state, &principal).await?;
                structured(&WhoAmI {
                    email: me.user.email,
                    name: me.user.name,
                    parcel_url: self.state.config.app_origin_str(),
                    scopes: principal.scopes.into_iter().collect(),
                })
            })
            .await
    }

    #[tool(
        description = "List parcels: your own (newest first, optionally filtered by `query`), or \
            with which=shared the ones other people shared with you. Paged: {items, total, has_more}.",
        annotations(read_only_hint = true)
    )]
    async fn list_parcels(
        &self,
        Parameters(p): Parameters<ListParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        ToolCall::new("list_parcels")
            .scopes([SCOPE_READ])
            .run(&ctx, |principal| async move {
                let me = caller(&self.state, &principal).await?;
                let limit = clamp_limit(p.limit);
                let offset = clamp_offset(p.offset);
                match p.which {
                    Which::Mine => {
                        let q = ListQuery {
                            q: p.query,
                            limit: Some(limit),
                            offset: Some(offset),
                        };
                        let list = parcel_api::list(&self.state, &me, &q)
                            .await
                            .map_err(tool_error)?;
                        structured(&Page::new(list.parcels, list.total, offset))
                    }
                    Which::Shared => {
                        let q = SharedQuery {
                            limit: Some(limit),
                            offset: Some(offset),
                        };
                        let list = sharing_api::shared_with_me(&self.state, &me, &q)
                            .await
                            .map_err(tool_error)?;
                        structured(&Page::new(list.parcels, list.total, offset))
                    }
                }
            })
            .await
    }

    #[tool(
        description = "A parcel's details: title, url, visibility, your access, its version \
            history and the current version's files. Works for parcels shared with you too.",
        annotations(read_only_hint = true)
    )]
    async fn get_parcel(
        &self,
        Parameters(p): Parameters<ParcelRef>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        ToolCall::new("get_parcel")
            .scopes([SCOPE_READ])
            .run(&ctx, |principal| async move {
                let me = caller(&self.state, &principal).await?;
                let (id, _) = parcel_ref(&p.parcel)?;
                let detail = parcel_api::detail(&self.state, Some(&me), &id)
                    .await
                    .map_err(tool_error)?;
                structured(&detail)
            })
            .await
    }

    #[tool(
        description = "One file's source from a parcel (default: the current version's entry \
            page): a JSON line describing it (path, version, content_type, size_bytes, \
            encoding), then the file itself as text, or as base64 when encoding is base64. Files \
            over 1 MiB are refused.",
        annotations(read_only_hint = true)
    )]
    async fn read_file(
        &self,
        Parameters(p): Parameters<ReadFileParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        ToolCall::new("read_file")
            .scopes([SCOPE_READ])
            .run(&ctx, |principal| async move {
                let me = caller(&self.state, &principal).await?;
                let (id, url_version) = parcel_ref(&p.parcel)?;
                // Also the access check: 404 unless the caller may read it.
                let detail = parcel_api::detail(&self.state, Some(&me), &id)
                    .await
                    .map_err(tool_error)?;
                let current = detail.parcel.current_version;
                let version = p.version.or(url_version).unwrap_or(current);
                let path = match p.path {
                    Some(path) => path,
                    None if version == current => detail.parcel.entry,
                    None => crate::db::parcels::version_entry(&self.state.db, &id, version)
                        .await
                        .map_err(ToolError::internal)?
                        .ok_or_else(|| ToolError::user(format!("version {version} not found")))?,
                };
                let file =
                    version_api::file_row(&self.state, Some(&me), &id, &version.to_string(), &path)
                        .await
                        .map_err(tool_error)?;
                if file.size_bytes > MAX_READ_BYTES {
                    return Err(ToolError::user(format!(
                        "{path:?} is {} bytes, more than this tool returns ({MAX_READ_BYTES}); \
                         use the parcel CLI (`parcel cat` / `parcel pull`) for large files",
                        file.size_bytes
                    )));
                }
                let bytes = version_api::load_file(&self.state, &file)
                    .await
                    .map_err(tool_error)?;
                let (encoding, content) = match std::str::from_utf8(&bytes) {
                    Ok(text) => (Encoding::Utf8, text.to_string()),
                    Err(_) => (
                        Encoding::Base64,
                        base64::engine::general_purpose::STANDARD.encode(&bytes),
                    ),
                };
                // The file as its own text block, so the model reads source
                // rather than a JSON-escaped string; what it is, before it.
                let about = FileInfo {
                    path: file.path,
                    version,
                    content_type: file.content_type,
                    size_bytes: bytes.len(),
                    encoding,
                };
                let about = serde_json::to_string(&about).map_err(ToolError::internal)?;
                Ok(CallToolResult::success(vec![
                    ContentBlock::text(about),
                    ContentBlock::text(content),
                ]))
            })
            .await
    }

    #[tool(
        description = "Publish a new parcel from files and get its private URL. Returns the \
            parcel (with `url`, the link to give the user) and version 1.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn publish_parcel(
        &self,
        Parameters(p): Parameters<PublishParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        ToolCall::new("publish_parcel")
            .scopes([SCOPE_WRITE])
            .timeout(PUBLISH_TIMEOUT)
            .run(&ctx, |principal| async move {
                let me = caller(&self.state, &principal).await?;
                crate::http::views::check_visibility(&self.state.config, p.visibility)
                    .map_err(tool_error)?;
                // Large requests already hold an upload permit (`upload_gate`).
                let uploads =
                    publish::collect_uploads(decode_files(p.files)?, &self.state.config.limits)
                        .map_err(tool_error)?;
                let meta = CreateParcelMetadata {
                    title: p.title,
                    description: p.description,
                    visibility: p.visibility,
                    entry: p.entry,
                    label: p.label,
                };
                let id = publish::create(&self.state, me.user.id, meta, uploads)
                    .await
                    .map_err(tool_error)?;
                tracing::info!(parcel = %id, user = %me.user.id, "parcel created through MCP");
                let result = parcel_api::publish_result(&self.state, &id, 1)
                    .await
                    .map_err(tool_error)?;
                structured(&result)
            })
            .await
    }

    #[tool(
        description = "Change a parcel you own. With `files`, `remove` or `entry` it publishes a \
            new version at the same URL (merge by default); `title`, `description` and \
            `visibility` change the parcel itself. Returns the parcel (and the new version, if any).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn update_parcel(
        &self,
        Parameters(p): Parameters<UpdateParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        ToolCall::new("update_parcel")
            .scopes([SCOPE_WRITE])
            .timeout(PUBLISH_TIMEOUT)
            .run(&ctx, |principal| async move {
                let me = caller(&self.state, &principal).await?;
                let (id, _) = parcel_ref(&p.parcel)?;
                let new_version = !p.files.is_empty() || !p.remove.is_empty() || p.entry.is_some();
                if !new_version {
                    if p.title.is_none() && p.description.is_none() && p.visibility.is_none() {
                        return Err(ToolError::user(
                            "nothing to change: pass files, remove, entry, title, description \
                             or visibility",
                        ));
                    }
                    // Only a new version has a mode, a label or a base to check.
                    if p.mode.is_some() || p.label.is_some() || p.base_version.is_some() {
                        return Err(ToolError::user(
                            "mode, label and base_version apply to a new version: pass files, \
                             remove or entry with them",
                        ));
                    }
                    let body = UpdateParcelRequest {
                        title: p.title,
                        description: p.description,
                        visibility: p.visibility,
                    };
                    let summary = parcel_api::update_metadata(&self.state, &me, &id, &body)
                        .await
                        .map_err(tool_error)?;
                    return structured(&serde_json::json!({ "parcel": summary }));
                }
                // Before touching the files, as the REST handler does.
                crate::http::views::owned(&self.state, &id, &me)
                    .await
                    .map_err(tool_error)?;
                crate::http::views::check_visibility(&self.state.config, p.visibility)
                    .map_err(tool_error)?;
                let uploads =
                    publish::collect_uploads(decode_files(p.files)?, &self.state.config.limits)
                        .map_err(tool_error)?;
                let meta = NewVersionMetadata {
                    mode: p.mode,
                    remove: p.remove,
                    entry: p.entry,
                    label: p.label,
                    title: p.title,
                    description: p.description,
                    visibility: p.visibility,
                    base_version: p.base_version,
                };
                let result = version_api::publish_version(&self.state, &me, &id, meta, uploads)
                    .await
                    .map_err(tool_error)?;
                structured(&result)
            })
            .await
    }

    #[tool(
        description = "Delete a parcel you own, with all its versions. Its URL stops working at \
            once. This cannot be undone: confirm with the user first.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true
        )
    )]
    async fn delete_parcel(
        &self,
        Parameters(p): Parameters<ParcelRef>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        ToolCall::new("delete_parcel")
            .scopes([SCOPE_WRITE])
            .run(&ctx, |principal| async move {
                let me = caller(&self.state, &principal).await?;
                let (id, _) = parcel_ref(&p.parcel)?;
                parcel_api::delete(&self.state, &me, &id)
                    .await
                    .map_err(tool_error)?;
                structured(&serde_json::json!({ "deleted": id }))
            })
            .await
    }

    #[tool(
        description = "Who a parcel you own is shared with (by email, with their role).",
        annotations(read_only_hint = true)
    )]
    async fn list_shares(
        &self,
        Parameters(p): Parameters<ParcelRef>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        ToolCall::new("list_shares")
            .scopes([SCOPE_READ])
            .run(&ctx, |principal| async move {
                let me = caller(&self.state, &principal).await?;
                let (id, _) = parcel_ref(&p.parcel)?;
                let list = sharing_api::list(&self.state, &me, &id)
                    .await
                    .map_err(tool_error)?;
                structured(&list)
            })
            .await
    }

    #[tool(
        description = "Share a parcel you own with someone by email, as a viewer (default) or \
            commenter, or change their role. They can open it when signed in with that email.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    async fn share_parcel(
        &self,
        Parameters(p): Parameters<ShareParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        ToolCall::new("share_parcel")
            .scopes([SCOPE_WRITE])
            .run(&ctx, |principal| async move {
                let me = caller(&self.state, &principal).await?;
                let (id, _) = parcel_ref(&p.parcel)?;
                let share =
                    sharing_api::put(&self.state, &me, &id, &p.email, p.role.unwrap_or_default())
                        .await
                        .map_err(tool_error)?;
                structured(&share)
            })
            .await
    }

    #[tool(
        description = "Stop sharing a parcel you own with someone. Pages they have open stop \
            loading at once.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true
        )
    )]
    async fn unshare_parcel(
        &self,
        Parameters(p): Parameters<UnshareParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        ToolCall::new("unshare_parcel")
            .scopes([SCOPE_WRITE])
            .run(&ctx, |principal| async move {
                let me = caller(&self.state, &principal).await?;
                let (id, _) = parcel_ref(&p.parcel)?;
                sharing_api::remove(&self.state, &me, &id, &p.email)
                    .await
                    .map_err(tool_error)?;
                structured(&serde_json::json!({ "unshared": p.email }))
            })
            .await
    }
}

#[tool_handler]
impl ServerHandler for ParcelMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("parcel", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}
