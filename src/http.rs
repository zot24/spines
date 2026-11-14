use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use utoipa::{OpenApi, ToSchema};

use crate::domain::{Item, ItemBody, ItemOut, NowPlaying, PluginId, ScanReport};
use crate::reconcile::jail;
use crate::{media_root, scan, Config, Store};

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    pub cfg: Config,
    pub now: Arc<Mutex<Option<NowPlaying>>>,
    pub reports: Arc<Mutex<Vec<ScanReport>>>,
}

impl AppState {
    pub fn new(store: Store, cfg: Config, reports: Vec<ScanReport>) -> Self {
        Self {
            store: Arc::new(store),
            cfg,
            now: Arc::new(Mutex::new(None)),
            reports: Arc::new(Mutex::new(reports)),
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub ok: bool,
    pub scans: Vec<ScanReport>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub id: PluginId,
    pub root_configured: bool,
    pub items: u32,
}

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct ItemQuery {
    /// `music` or `books`. Omit it to list both shelves.
    plugin: Option<String>,
    /// Every whitespace-separated word must appear.
    q: Option<String>,
}

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct ForceQuery {
    /// When true, read every file again even if the stamp matches.
    force: Option<bool>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/plugins", get(plugins))
        .route("/api/items", get(list_items))
        .route("/api/items/{id}", get(get_item))
        .route("/api/items/{id}/cover", get(cover))
        .route("/api/items/{id}/spine", get(spine_image))
        .route("/api/items/{id}/audio/{index}", get(audio))
        .route("/api/now-playing", get(get_now).post(post_now))
        .route("/api/rescan", post(rescan))
        .route("/openapi.json", get(openapi))
        .route("/mcp", post(mcp_http))
        .route("/", get(index))
        .with_state(state)
}

pub fn openapi_json() -> String {
    let mut spec = ApiDoc::openapi();
    spec.info.description = Some(
        "Self-hosted media library. The same types drive this document, the web UI, and MCP."
            .into(),
    );
    format!(
        "{}\n",
        spec.to_pretty_json().unwrap_or_else(|_| "{}".into())
    )
}

#[derive(OpenApi)]
#[openapi(
    info(title = "Spines", version = "0.1.0"),
    paths(
        health,
        plugins,
        list_items,
        get_item,
        cover,
        spine_image,
        audio,
        get_now,
        post_now,
        rescan
    ),
    components(schemas(
        Health,
        PluginInfo,
        ItemOut,
        crate::domain::PublicBody,
        crate::domain::PublicAlbum,
        crate::domain::PublicBook,
        crate::domain::PublicTrack,
        crate::domain::Palette,
        crate::domain::SpineStyle,
        PluginId,
        crate::domain::ViewName,
        NowPlaying,
        ScanReport
    )),
    tags((name = "library", description = "Catalog and playback"))
)]
struct ApiDoc;

#[utoipa::path(get, path = "/api/health", responses((status = 200, description = "The process has finished its startup scan", body = Health)))]
async fn health(State(state): State<AppState>) -> Json<Health> {
    let scans = state
        .reports
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    Json(Health { ok: true, scans })
}

#[utoipa::path(get, path = "/api/plugins", responses((status = 200, description = "Shelves this process knows about", body = Vec<PluginInfo>)))]
async fn plugins(State(state): State<AppState>) -> Response {
    let mut out = Vec::new();
    for id in [PluginId::Music, PluginId::Books] {
        let items = match state.store.items(Some(id), &[]) {
            Ok(items) => items,
            Err(err) => return fail(err),
        };
        out.push(PluginInfo {
            id,
            root_configured: media_root(&state.cfg, id).is_some(),
            items: items.len() as u32,
        });
    }
    Json(out).into_response()
}

#[utoipa::path(
    get,
    path = "/api/items",
    params(ItemQuery),
    responses(
        (status = 200, description = "Items, ordered by shelf sort key", body = Vec<ItemOut>),
        (status = 400, description = "Unknown plugin id")
    )
)]
async fn list_items(State(state): State<AppState>, Query(query): Query<ItemQuery>) -> Response {
    let plugin = match query
        .plugin
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => None,
        Some(name) => match PluginId::parse(name) {
            Some(id) => Some(id),
            None => return (StatusCode::BAD_REQUEST, "unknown plugin").into_response(),
        },
    };
    let words: Vec<String> = query
        .q
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    match state.store.items(plugin, &words) {
        Ok(items) => Json(items.iter().map(ItemOut::from).collect::<Vec<_>>()).into_response(),
        Err(err) => fail(err),
    }
}

#[utoipa::path(
    get,
    path = "/api/items/{id}",
    params(("id", description = "Catalog id")),
    responses(
        (status = 200, description = "One item", body = ItemOut),
        (status = 404, description = "No such item")
    )
)]
async fn get_item(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match load(&state, &id) {
        Ok(Some(item)) => Json(ItemOut::from(&item)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(err) => fail(err),
    }
}

#[utoipa::path(
    get,
    path = "/api/items/{id}/cover",
    params(("id", description = "Catalog id")),
    responses(
        (status = 200, description = "JPEG cover", content_type = "image/jpeg"),
        (status = 404, description = "No cover")
    )
)]
async fn cover(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let item = match load(&state, &id) {
        Ok(Some(item)) => item,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(err) => return fail(err),
    };
    let Some(path) = cover_file(&state, &item) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match std::fs::read(&path) {
        Ok(bytes) => file_response(&bytes, image_type(&bytes), None),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/api/items/{id}/spine",
    params(("id", description = "Catalog id")),
    responses(
        (status = 200, description = "Generated spine PNG", content_type = "image/png"),
        (status = 404, description = "No spine")
    )
)]
async fn spine_image(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    if !safe_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = state.store.spine_path(&id);
    match std::fs::read(&path) {
        Ok(bytes) => file_response(&bytes, "image/png", None),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/api/items/{id}/audio/{index}",
    params(
        ("id", description = "Album id"),
        ("index", description = "Track index inside the album")
    ),
    responses(
        (status = 200, description = "Whole audio file"),
        (status = 206, description = "Byte range"),
        (status = 404, description = "No such track"),
        (status = 416, description = "Range is outside the file")
    )
)]
async fn audio(
    State(state): State<AppState>,
    Path((id, index)): Path<(String, u32)>,
    headers: HeaderMap,
) -> Response {
    let item = match load(&state, &id) {
        Ok(Some(item)) => item,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(err) => return fail(err),
    };
    let ItemBody::Album(album) = &item.body else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(track) = album.tracks.iter().find(|t| t.index == index) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(root) = state.cfg.music.as_deref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(path) = jail(root, &track.rel_path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    file_response(&bytes, mime_of(&path), range)
}

#[utoipa::path(
    get,
    path = "/api/now-playing",
    responses((status = 200, description = "What this process is playing, or null", body = Option<NowPlaying>))
)]
async fn get_now(State(state): State<AppState>) -> Response {
    let now = state.now.lock().unwrap_or_else(|p| p.into_inner()).clone();
    json_no_store(&now)
}

#[utoipa::path(
    post,
    path = "/api/now-playing",
    request_body = NowPlaying,
    responses((status = 200, description = "Stored for this process only", body = NowPlaying))
)]
async fn post_now(State(state): State<AppState>, Json(body): Json<NowPlaying>) -> Response {
    if body.item_id.trim().is_empty() || !safe_id(&body.item_id) {
        return (StatusCode::BAD_REQUEST, "itemId is required").into_response();
    }
    *state.now.lock().unwrap_or_else(|p| p.into_inner()) = Some(body.clone());
    json_no_store(&body)
}

#[utoipa::path(
    post,
    path = "/api/rescan",
    params(ForceQuery),
    responses((status = 200, description = "Scan reports, one per shelf", body = Vec<ScanReport>))
)]
async fn rescan(State(state): State<AppState>, Query(query): Query<ForceQuery>) -> Response {
    match scan(&state.store, &state.cfg, query.force.unwrap_or(false)) {
        Ok(reports) => {
            *state.reports.lock().unwrap_or_else(|p| p.into_inner()) = reports.clone();
            Json(reports).into_response()
        }
        Err(err) => fail(err),
    }
}

async fn openapi() -> Response {
    ([(header::CONTENT_TYPE, "application/json")], openapi_json()).into_response()
}

async fn mcp_http(State(state): State<AppState>, body: String) -> Response {
    let now = state.now.lock().unwrap_or_else(|p| p.into_inner()).clone();
    match crate::mcp::reply(&state.store, now.as_ref(), &body) {
        crate::mcp::Reply::Response(json) => {
            ([(header::CONTENT_TYPE, "application/json")], json).into_response()
        }
        crate::mcp::Reply::Notification => StatusCode::ACCEPTED.into_response(),
        crate::mcp::Reply::Bad(message) => (StatusCode::BAD_REQUEST, message).into_response(),
    }
}

async fn index() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("../ui/index.html"),
    )
        .into_response()
}

fn load(state: &AppState, id: &str) -> crate::Result<Option<Item>> {
    if !safe_id(id) {
        return Ok(None);
    }
    state.store.item(id)
}

fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn cover_file(state: &AppState, item: &Item) -> Option<std::path::PathBuf> {
    match &item.body {
        ItemBody::Album(_) => {
            let path = state.store.cover_path(&item.id);
            path.is_file().then_some(path)
        }
        ItemBody::Book(book) => {
            let root = state.cfg.books.as_deref()?;
            let rel = book.cover_rel.as_deref()?;
            jail(root, rel).filter(|p| p.is_file())
        }
    }
}

fn image_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else {
        "image/jpeg"
    }
}

fn mime_of(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "m4a" | "aac" => "audio/mp4",
        "aif" | "aiff" => "audio/aiff",
        _ => "application/octet-stream",
    }
}

fn file_response(bytes: &[u8], mime: &str, range: Option<&str>) -> Response {
    let total = bytes.len() as u64;
    if let Some(range_header) = range {
        return match byte_range(range_header, total) {
            Ok((start, end)) => {
                let slice = bytes[start as usize..=end as usize].to_vec();
                Response::builder()
                    .status(StatusCode::PARTIAL_CONTENT)
                    .header(header::CONTENT_TYPE, mime)
                    .header(header::ACCEPT_RANGES, "bytes")
                    .header(
                        header::CONTENT_RANGE,
                        format!("bytes {start}-{end}/{total}"),
                    )
                    .header(header::CONTENT_LENGTH, slice.len())
                    .body(Body::from(slice))
                    .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
            }
            Err(()) => Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{total}"))
                .body(Body::empty())
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        };
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, bytes.len())
        .body(Body::from(bytes.to_vec()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn byte_range(header: &str, len: u64) -> std::result::Result<(u64, u64), ()> {
    let spec = header.trim().strip_prefix("bytes=").ok_or(())?;
    let spec = spec.split(',').next().unwrap_or("").trim();
    if len == 0 {
        return Err(());
    }
    if let Some(suffix) = spec.strip_prefix('-') {
        let n: u64 = suffix.parse().map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        let n = n.min(len);
        return Ok((len - n, len - 1));
    }
    let (start_s, end_s) = spec.split_once('-').ok_or(())?;
    let start: u64 = start_s.parse().map_err(|_| ())?;
    if start >= len {
        return Err(());
    }
    let end = if end_s.is_empty() {
        len - 1
    } else {
        end_s.parse::<u64>().map_err(|_| ())?.min(len - 1)
    };
    if end < start {
        return Err(());
    }
    Ok((start, end))
}

fn json_no_store<T: Serialize>(value: &T) -> Response {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"null".to_vec());
    Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn fail(err: crate::Error) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response()
}
