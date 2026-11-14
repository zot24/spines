use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Which shelf an item belongs to. One closed set. A new shelf is a new variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum PluginId {
    Music,
    Books,
}

impl PluginId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Music => "music",
            Self::Books => "books",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "music" => Some(Self::Music),
            "books" => Some(Self::Books),
            _ => None,
        }
    }
}

/// How the web UI opens an item. The value travels with the item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ViewName {
    CdWall,
    BookPlanks,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Palette {
    pub bg: String,
    pub fg: String,
    pub accent: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpineStyle {
    pub width: i32,
    pub height: f64,
    pub font: String,
    pub caps: bool,
    pub title_size: i32,
    pub lines: u8,
    pub mark: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub title: String,
    pub duration_ms: u64,
    pub disc: u32,
    pub number: u32,
    /// Index into the album's track list. Playback uses this.
    pub index: u32,
    /// Path relative to the music root. The server jails it. The wall does not show it.
    pub rel_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AlbumBody {
    pub artist: String,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BookBody {
    pub calibre_id: i64,
    pub authors: Vec<String>,
    pub publisher: Option<String>,
    pub series: Option<String>,
    pub series_index: Option<f64>,
    pub pages: Option<i32>,
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub formats: Vec<String>,
    pub palette: Palette,
    pub spine: SpineStyle,
    /// Cover path relative to the books root. Empty when the book has no cover.
    pub cover_rel: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ItemBody {
    Album(AlbumBody),
    Book(Box<BookBody>),
}

/// One catalog row. Stored as JSON. URLs are added when the row crosses HTTP.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub plugin: PluginId,
    pub view: ViewName,
    pub title: String,
    pub subtitle: String,
    pub year: Option<i32>,
    pub sort_key: String,
    pub haystack: String,
    pub body: ItemBody,
}

impl Item {
    pub fn view_for(body: &ItemBody) -> ViewName {
        match body {
            ItemBody::Album(_) => ViewName::CdWall,
            ItemBody::Book(_) => ViewName::BookPlanks,
        }
    }
}

/// Track fields the browser is allowed to see. The file path stays in the catalog.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PublicTrack {
    pub title: String,
    pub duration_ms: u64,
    pub disc: u32,
    pub number: u32,
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PublicAlbum {
    pub artist: String,
    pub tracks: Vec<PublicTrack>,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PublicBook {
    pub calibre_id: i64,
    pub authors: Vec<String>,
    pub publisher: Option<String>,
    pub series: Option<String>,
    pub series_index: Option<f64>,
    pub pages: Option<i32>,
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub formats: Vec<String>,
    pub palette: Palette,
    pub spine: SpineStyle,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PublicBody {
    Album(PublicAlbum),
    Book(Box<PublicBook>),
}

/// Catalog row as it crosses HTTP. Paths and the search haystack stay in SQLite.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemOut {
    pub id: String,
    pub plugin: PluginId,
    pub view: ViewName,
    pub title: String,
    pub subtitle: String,
    pub year: Option<i32>,
    pub sort_key: String,
    pub body: PublicBody,
}

impl From<&Item> for ItemOut {
    fn from(item: &Item) -> Self {
        let body = match &item.body {
            ItemBody::Album(album) => PublicBody::Album(PublicAlbum {
                artist: album.artist.clone(),
                tracks: album
                    .tracks
                    .iter()
                    .map(|t| PublicTrack {
                        title: t.title.clone(),
                        duration_ms: t.duration_ms,
                        disc: t.disc,
                        number: t.number,
                        index: t.index,
                    })
                    .collect(),
            }),
            ItemBody::Book(book) => PublicBody::Book(Box::new(PublicBook {
                calibre_id: book.calibre_id,
                authors: book.authors.clone(),
                publisher: book.publisher.clone(),
                series: book.series.clone(),
                series_index: book.series_index,
                pages: book.pages,
                tags: book.tags.clone(),
                description: book.description.clone(),
                formats: book.formats.clone(),
                palette: book.palette.clone(),
                spine: book.spine.clone(),
            })),
        };
        Self {
            id: item.id.clone(),
            plugin: item.plugin,
            view: item.view,
            title: item.title.clone(),
            subtitle: item.subtitle.clone(),
            year: item.year,
            sort_key: item.sort_key.clone(),
            body,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub plugin: String,
    pub changed: u32,
    pub unchanged: u32,
    pub removed: u32,
    pub elapsed_ms: u128,
    pub skipped: bool,
    pub reason: String,
}

impl ScanReport {
    pub fn log_line(&self) -> String {
        format!(
            "scan plugin={} changed={} unchanged={} removed={} elapsed_ms={} skipped={} reason={}",
            self.plugin,
            self.changed,
            self.unchanged,
            self.removed,
            self.elapsed_ms,
            self.skipped,
            self.reason
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    pub item_id: String,
    pub title: String,
    pub subtitle: String,
    pub track_title: String,
    pub track_index: u32,
    pub position_ms: u64,
    pub paused: bool,
}
