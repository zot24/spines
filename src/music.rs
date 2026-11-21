use std::fs;
use std::path::Path;

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::PictureType;
use lofty::probe::read_from_path;
use lofty::tag::{Accessor, ItemKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::{AlbumBody, Item, ItemBody, PluginId, Track, ViewName};
use crate::error::Result;
use crate::plugin::{AssembleCtx, Assembled, ParsedPart, Plan, Plugin, SourceStat};
use crate::reconcile::{mtime_ns, rel_key};
use crate::spineimg::{jpeg_bytes, spine_png};

const AUDIO: &[&str] = &[
    "mp3", "flac", "ogg", "opus", "wav", "m4a", "aac", "aiff", "aif",
];
const SIDECAR: &[&str] = &[
    "cover.jpg",
    "cover.jpeg",
    "folder.jpg",
    "cover.png",
    "folder.png",
];

pub struct Music;

#[derive(Debug, Serialize, Deserialize)]
struct TrackPart {
    album_id: String,
    artist: String,
    album: String,
    title: String,
    track: u32,
    disc: u32,
    duration_ms: u64,
    year: Option<i32>,
    rel_path: String,
    art_file: Option<String>,
}

impl Plugin for Music {
    fn id(&self) -> PluginId {
        PluginId::Music
    }

    fn plan(&self, root: &Path) -> Result<Plan> {
        let mut sources = Vec::new();
        walk(root, root, &mut sources)?;
        sources.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(Plan::Sources(sources))
    }

    fn parse(&self, root: &Path, source: &SourceStat, cache: &Path) -> Result<Vec<ParsedPart>> {
        let rel = rel_key(root, &source.abs).unwrap_or_else(|| source.key.clone());
        let parent = source.abs.parent();
        let folder_album = parent
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Unknown album".into());
        let folder_artist = parent
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .filter(|_| p_not_root(root, parent.and_then(|p| p.parent()).unwrap_or(root)))
            .unwrap_or_else(|| "Unknown artist".into());
        let stem = source
            .abs
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".into());

        let mut artist = folder_artist;
        let mut album = folder_album;
        let mut title = stem.clone();
        let mut track = leading_number(&stem).unwrap_or(1);
        let mut disc = 1u32;
        let mut duration_ms = 0u64;
        let mut year = None;
        let mut art: Option<Vec<u8>> = None;

        if let Ok(tagged) = read_from_path(&source.abs) {
            duration_ms = u64::try_from(tagged.properties().duration().as_millis()).unwrap_or(0);
            if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
                if let Some(v) = tag
                    .artist()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                {
                    artist = v;
                }
                if let Some(v) = tag
                    .get_string(ItemKey::AlbumArtist)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    artist = v.to_string();
                }
                if let Some(v) = tag
                    .album()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                {
                    album = v;
                }
                if let Some(v) = tag
                    .title()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                {
                    title = v;
                }
                if let Some(n) = tag.track() {
                    track = n;
                }
                if let Some(n) = tag.disk() {
                    disc = n;
                }
                year = tag
                    .get_string(ItemKey::RecordingDate)
                    .or_else(|| tag.get_string(ItemKey::Year))
                    .and_then(year_of);
                art = tag
                    .pictures()
                    .iter()
                    .find(|p| p.pic_type() == PictureType::CoverFront)
                    .or_else(|| tag.pictures().first())
                    .map(|p| p.data().to_vec());
            }
        }
        if art.is_none() {
            if let Some(dir) = parent {
                for name in SIDECAR {
                    let path = dir.join(name);
                    if let Ok(bytes) = fs::read(&path) {
                        art = Some(bytes);
                        break;
                    }
                }
            }
        }
        let art_file = if let Some(jpeg) = art.as_deref().and_then(jpeg_bytes) {
            let mut hasher = Sha256::new();
            hasher.update(&jpeg);
            let name = format!("{}.jpg", hex::encode(hasher.finalize()));
            fs::write(cache.join(&name), &jpeg)?;
            Some(name)
        } else {
            None
        };
        let album_id = album_item_id(&artist, &album);
        let part = TrackPart {
            album_id: album_id.clone(),
            artist,
            album,
            title,
            track,
            disc,
            duration_ms,
            year,
            rel_path: rel,
            art_file,
        };
        Ok(vec![ParsedPart {
            item_id: album_id,
            body: serde_json::to_value(part)?,
        }])
    }

    fn assemble(&self, ctx: AssembleCtx<'_>) -> Result<Assembled> {
        let mut tracks: Vec<TrackPart> = ctx
            .bodies
            .iter()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .collect();
        tracks.sort_by(|a, b| {
            a.disc
                .cmp(&b.disc)
                .then(a.track.cmp(&b.track))
                .then(a.rel_path.cmp(&b.rel_path))
        });
        let artist = tracks
            .first()
            .map(|t| t.artist.clone())
            .unwrap_or_else(|| "Unknown artist".into());
        let album = tracks
            .first()
            .map(|t| t.album.clone())
            .unwrap_or_else(|| "Unknown album".into());
        let year = tracks.iter().find_map(|t| t.year);
        let cover = tracks.iter().find_map(|t| {
            let name = t.art_file.as_ref()?;
            fs::read(ctx.cache_dir.join(name)).ok()
        });
        let api_tracks = tracks
            .iter()
            .enumerate()
            .map(|(i, t)| Track {
                title: t.title.clone(),
                duration_ms: t.duration_ms,
                disc: t.disc,
                number: t.track,
                index: i as u32,
                rel_path: t.rel_path.clone(),
            })
            .collect::<Vec<_>>();
        let hay = {
            let mut s = format!("{} {}", artist, album);
            for t in &api_tracks {
                s.push(' ');
                s.push_str(&t.title);
            }
            s.to_lowercase()
        };
        let fallback = hash_color(ctx.item_id);
        let spine = spine_png(cover.as_deref(), fallback)?;
        let item = Item {
            id: ctx.item_id.to_string(),
            plugin: PluginId::Music,
            view: ViewName::CdWall,
            title: album,
            subtitle: artist.clone(),
            year,
            sort_key: format!(
                "{} {}",
                artist,
                tracks.first().map(|t| t.album.as_str()).unwrap_or("")
            )
            .to_lowercase(),
            haystack: hay,
            body: ItemBody::Album(AlbumBody {
                artist,
                tracks: api_tracks,
            }),
        };
        Ok(Assembled {
            item,
            cover_jpeg: cover,
            spine_png: spine,
        })
    }
}

fn p_not_root(root: &Path, path: &Path) -> bool {
    path != root
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<SourceStat>) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            walk(root, &path, out)?;
            continue;
        }
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !AUDIO.contains(&ext.as_str()) {
            continue;
        }
        let meta = entry.metadata()?;
        let Some(key) = rel_key(root, &path) else {
            continue;
        };
        out.push(SourceStat {
            key,
            abs: path,
            mtime_ns: mtime_ns(&meta),
            size: meta.len(),
        });
    }
    Ok(())
}

fn year_of(value: &str) -> Option<i32> {
    let digits: String = value.chars().take(4).collect();
    let year: i32 = digits.parse().ok()?;
    if year > 0 {
        Some(year)
    } else {
        None
    }
}

fn leading_number(stem: &str) -> Option<u32> {
    let digits: String = stem.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

pub fn album_item_id(artist: &str, album: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(artist.trim().to_lowercase().as_bytes());
    hasher.update(b"\n");
    hasher.update(album.trim().to_lowercase().as_bytes());
    format!("mus_{}", hex::encode(hasher.finalize()))
}

fn hash_color(id: &str) -> [u8; 3] {
    let mut hasher = Sha256::new();
    hasher.update(id.as_bytes());
    let d = hasher.finalize();
    [d[0], d[1], d[2]]
}
