use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::bookstyle::spine_style;
use crate::domain::{BookBody, Item, ItemBody, Palette, PluginId, ViewName};
use crate::error::{Error, Result};
use crate::palette::{fallback_palette, palette_from_rgba};
use crate::plugin::{AssembleCtx, Assembled, ParsedPart, Plan, Plugin, SourceStat};
use crate::reconcile::{jail, mtime_ns, rel_key};
use crate::spineimg::spine_png;

pub struct Books;

#[derive(Debug, Serialize, Deserialize)]
struct BookPart {
    calibre_id: i64,
    title: String,
    authors: Vec<String>,
    publisher: Option<String>,
    series: Option<String>,
    series_index: Option<f64>,
    year: Option<i32>,
    pages: Option<i32>,
    tags: Vec<String>,
    description: Option<String>,
    formats: Vec<String>,
    cover_rel: Option<String>,
    author_sort: String,
    title_sort: String,
    palette: Palette,
}

impl Plugin for Books {
    fn id(&self) -> PluginId {
        PluginId::Books
    }

    fn plan(&self, root: &Path) -> Result<Plan> {
        let Some(db) = find_db(root) else {
            return Ok(Plan::Absent);
        };
        let meta = fs::metadata(&db)?;
        let Some(key) = rel_key(root, &db) else {
            return Ok(Plan::Absent);
        };
        Ok(Plan::Sources(vec![SourceStat {
            key,
            abs: db,
            mtime_ns: mtime_ns(&meta),
            size: meta.len(),
        }]))
    }

    fn parse(&self, root: &Path, source: &SourceStat, _cache: &Path) -> Result<Vec<ParsedPart>> {
        let db_dir = source.abs.parent().unwrap_or(root);
        let conn = open_readonly(&source.abs)?;
        let with_pages = table_exists(&conn, "books_pages_link")?;
        let sql = list_sql(with_pages);
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(Raw {
                id: row.get(0)?,
                title: row.get(1)?,
                pubdate: row.get(2)?,
                series_index: row.get(3)?,
                authors: row.get(4)?,
                tags: row.get(5)?,
                formats: row.get(6)?,
                publisher: row.get(7)?,
                series: row.get(8)?,
                comments: row.get(9)?,
                pages: row.get(10)?,
                path: row.get(11)?,
                has_cover: row.get(12)?,
                author_sort: row.get(13)?,
                title_sort: row.get(14)?,
            })
        })?;
        let mut parts = Vec::new();
        for row in rows {
            let row = row?;
            let book = to_part(root, db_dir, row)?;
            let item_id = format!("bok_{}", book.calibre_id);
            parts.push(ParsedPart {
                item_id,
                body: serde_json::to_value(book)?,
            });
        }
        Ok(parts)
    }

    fn assemble(&self, ctx: AssembleCtx<'_>) -> Result<Assembled> {
        let part: BookPart = serde_json::from_value(
            ctx.bodies
                .first()
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        )?;
        let spine = spine_style(part.calibre_id, &part.title, part.pages);
        let cover_bytes = part
            .cover_rel
            .as_deref()
            .and_then(|rel| jail(ctx.media_root, rel))
            .and_then(|p| fs::read(p).ok());
        let rgb = parse_hex(&part.palette.bg).unwrap_or([40, 36, 32]);
        let spine_png = spine_png(cover_bytes.as_deref(), rgb)?;
        let authors = part.authors.clone();
        let subtitle = authors.join(", ");
        let mut hay = format!("{} {}", part.title, subtitle);
        for extra in [&part.series, &part.publisher].into_iter().flatten() {
            hay.push(' ');
            hay.push_str(extra);
        }
        for tag in &part.tags {
            hay.push(' ');
            hay.push_str(tag);
        }
        if let Some(desc) = &part.description {
            hay.push(' ');
            hay.push_str(desc);
        }
        let sort_key = format!(
            "{}\n{}\n{:08}",
            part.author_sort, part.title_sort, part.calibre_id
        );
        let year = part.year;
        let title = part.title.clone();
        let item = Item {
            id: ctx.item_id.to_string(),
            plugin: PluginId::Books,
            view: ViewName::BookPlanks,
            title,
            subtitle,
            year,
            sort_key,
            haystack: hay.to_lowercase(),
            body: ItemBody::Book(Box::new(BookBody {
                calibre_id: part.calibre_id,
                authors,
                publisher: part.publisher,
                series: part.series,
                series_index: part.series_index,
                pages: part.pages,
                tags: part.tags,
                description: part.description,
                formats: part.formats,
                palette: part.palette,
                spine,
                cover_rel: part.cover_rel,
            })),
        };
        Ok(Assembled {
            item,
            cover_jpeg: None,
            spine_png,
        })
    }
}

struct Raw {
    id: i64,
    title: Option<String>,
    pubdate: Option<String>,
    series_index: Option<f64>,
    authors: Option<String>,
    tags: Option<String>,
    formats: Option<String>,
    publisher: Option<String>,
    series: Option<String>,
    comments: Option<String>,
    pages: Option<i64>,
    path: Option<String>,
    has_cover: Option<i64>,
    author_sort: Option<String>,
    title_sort: Option<String>,
}

fn to_part(root: &Path, db_dir: &Path, row: Raw) -> Result<BookPart> {
    let authors = split_unit(row.authors)
        .into_iter()
        .filter(|a| !a.is_empty() && a != "Unknown")
        .collect::<Vec<_>>();
    let tags = split_unit(row.tags);
    let formats = split_unit(row.formats)
        .into_iter()
        .map(|f| f.to_lowercase())
        .collect::<Vec<_>>();
    let series = empty_to_none(row.series);
    let description = row
        .comments
        .as_deref()
        .map(html_to_text)
        .filter(|s| !s.is_empty());
    let pages = row.pages.filter(|n| *n > 0).map(|n| n as i32);
    let cover_rel = if row.has_cover.unwrap_or(0) != 0 {
        row.path.as_ref().and_then(|book_path| {
            let abs = db_dir.join(book_path).join("cover.jpg");
            rel_key(root, &abs)
        })
    } else {
        None
    };
    let palette = match cover_rel
        .as_deref()
        .and_then(|rel| jail(root, rel))
        .and_then(|p| fs::read(p).ok())
    {
        Some(bytes) => match image::load_from_memory(&bytes) {
            Ok(img) => palette_from_rgba(img.to_rgba8().as_raw()),
            Err(_) => fallback_palette(row.id),
        },
        None => fallback_palette(row.id),
    };
    let title = row.title.unwrap_or_default().trim().to_string();
    Ok(BookPart {
        calibre_id: row.id,
        title: if title.is_empty() {
            "Untitled".into()
        } else {
            title
        },
        authors,
        publisher: empty_to_none(row.publisher),
        series: series.clone(),
        series_index: if series.is_some() {
            row.series_index
        } else {
            None
        },
        year: row.pubdate.as_deref().and_then(year_from_pubdate),
        pages,
        tags,
        description,
        formats,
        cover_rel,
        author_sort: row.author_sort.unwrap_or_default(),
        title_sort: row.title_sort.unwrap_or_default(),
        palette,
    })
}

pub fn find_db(root: &Path) -> Option<PathBuf> {
    let direct = root.join("metadata.db");
    if direct.is_file() {
        return Some(direct);
    }
    let mut children = fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok())
        .collect::<Vec<_>>();
    children.sort_by_key(|e| e.file_name());
    for entry in children {
        let candidate = entry.path().join("metadata.db");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn open_readonly(path: &Path) -> Result<Connection> {
    let url = url::Url::from_file_path(path)
        .map_err(|_| Error::msg("metadata.db path is not absolute"))?;
    let uri = format!("{}?mode=ro&immutable=1", url.as_str());
    Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(Error::from)
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

fn list_sql(with_pages: bool) -> String {
    let pages = if with_pages {
        "(SELECT MAX(pages) FROM books_pages_link WHERE book = b.id)"
    } else {
        "NULL"
    };
    format!(
        "
        SELECT b.id, b.title, b.pubdate, b.series_index,
          (SELECT group_concat(v, char(31)) FROM (SELECT a.name AS v FROM books_authors_link l JOIN authors a ON a.id = l.author WHERE l.book = b.id ORDER BY l.id)) AS authors,
          (SELECT group_concat(v, char(31)) FROM (SELECT t.name AS v FROM books_tags_link l JOIN tags t ON t.id = l.tag WHERE l.book = b.id ORDER BY l.id)) AS tags,
          (SELECT group_concat(v, char(31)) FROM (SELECT upper(l.format) AS v FROM data l WHERE l.book = b.id ORDER BY l.id)) AS formats,
          (SELECT p.name FROM books_publishers_link l JOIN publishers p ON p.id = l.publisher WHERE l.book = b.id) AS publisher,
          (SELECT s.name FROM books_series_link l JOIN series s ON s.id = l.series WHERE l.book = b.id) AS series,
          (SELECT c.text FROM comments c WHERE c.book = b.id) AS comments,
          {pages} AS pages,
          b.path, b.has_cover, b.author_sort, b.sort
        FROM books b
        ORDER BY b.author_sort COLLATE NOCASE, b.sort COLLATE NOCASE, b.id
        "
    )
}

fn split_unit(s: Option<String>) -> Vec<String> {
    match s {
        Some(s) if !s.is_empty() => s.split('\u{1f}').map(|p| p.to_string()).collect(),
        _ => Vec::new(),
    }
}

fn empty_to_none(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub fn year_from_pubdate(s: &str) -> Option<i32> {
    let digits: String = s.chars().take(4).collect();
    let year: i32 = digits.parse().ok()?;
    if year > 101 {
        Some(year)
    } else {
        None
    }
}

pub fn html_to_text(html: &str) -> String {
    let breaks = regex_break().replace_all(html, "\n");
    let no_tags = regex_tags().replace_all(&breaks, "");
    let decoded = decode_entities(&no_tags);
    let collapsed = regex_blank().replace_all(decoded.trim(), "\n\n");
    collapsed.trim().to_string()
}

fn regex_break() -> &'static regex::Regex {
    static RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)</(p|div|h\d|li)>|<br\s*/?>").unwrap());
    &RE
}

fn regex_tags() -> &'static regex::Regex {
    static RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"<[^>]*>").unwrap());
    &RE
}

fn regex_blank() -> &'static regex::Regex {
    static RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\n{3,}").unwrap());
    &RE
}

fn decode_entities(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        let Some(end) = rest.find(';') else {
            out.push('&');
            break;
        };
        let entity = &rest[..end];
        rest = &rest[end + 1..];
        if let Some(ch) = entity_char(entity) {
            out.push(ch);
        } else {
            out.push('&');
            out.push_str(entity);
            out.push(';');
        }
    }
    out.push_str(rest);
    out
}

fn entity_char(entity: &str) -> Option<char> {
    match entity.to_ascii_lowercase().as_str() {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => {
            if let Some(hex) = entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
            {
                return char::from_u32(u32::from_str_radix(hex, 16).ok()?);
            }
            if let Some(num) = entity.strip_prefix('#') {
                return char::from_u32(num.parse().ok()?);
            }
            None
        }
    }
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    Some([
        u8::from_str_radix(&s[0..2], 16).ok()?,
        u8::from_str_radix(&s[2..4], 16).ok()?,
        u8::from_str_radix(&s[4..6], 16).ok()?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_and_year_match_the_bookshelf_rules() {
        assert_eq!(
            html_to_text("<p>Anne &amp; Wentworth</p>"),
            "Anne & Wentworth"
        );
        assert_eq!(year_from_pubdate("1994-01-01 00:00:00+00:00"), Some(1994));
        assert_eq!(year_from_pubdate("0101-01-01 00:00:00+00:00"), None);
    }
}
