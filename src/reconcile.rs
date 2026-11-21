use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::Instant;

use sha2::{Digest, Sha256};

use crate::domain::{PluginId, ScanReport};
use crate::error::Result;
use crate::plugin::{Plan, Plugin};
use crate::store::{NewPart, PartRow, Store};

pub fn reconcile(
    store: &Store,
    plugin: &dyn Plugin,
    root: Option<&Path>,
    force: bool,
) -> Result<ScanReport> {
    let started = Instant::now();
    let id = plugin.id();
    let Some(root) = root.filter(|p| p.is_dir()) else {
        return Ok(finish(id, 0, 0, 0, started, true, "missing-root"));
    };
    let plan = plugin.plan(root)?;
    let sources = match plan {
        Plan::Absent => return Ok(finish(id, 0, 0, 0, started, true, "missing-root")),
        Plan::Sources(sources) => sources,
    };
    let stamp = stamp_of(&sources);
    if !force && store.stamp(id)?.as_deref() == Some(stamp.as_str()) {
        return Ok(finish(
            id,
            0,
            sources.len() as u32,
            0,
            started,
            true,
            "stamp",
        ));
    }

    let old = store.parts(id)?;
    let mut old_by_key: HashMap<String, Vec<PartRow>> = HashMap::new();
    for part in old {
        old_by_key
            .entry(part.source_key.clone())
            .or_default()
            .push(part);
    }

    let mut final_parts: Vec<NewPart> = Vec::new();
    let mut changed = 0u32;
    let mut unchanged = 0u32;
    let mut touched_items: HashSet<String> = HashSet::new();

    for source in &sources {
        let previous = old_by_key.get(&source.key);
        let same_stat = previous
            .and_then(|rows| rows.first())
            .is_some_and(|row| row.mtime_ns == source.mtime_ns && row.size == source.size);
        if same_stat && !force {
            for row in previous.unwrap() {
                final_parts.push(new_from_old(row));
            }
            unchanged += 1;
            continue;
        }
        let hash = hash_file(&source.abs)?;
        let same_bytes = previous
            .and_then(|rows| rows.first())
            .is_some_and(|row| row.content_hash == hash);
        if same_bytes && !force {
            for row in previous.unwrap() {
                let mut kept = new_from_old(row);
                kept.mtime_ns = source.mtime_ns;
                kept.size = source.size;
                final_parts.push(kept);
            }
            unchanged += 1;
            continue;
        }
        if let Some(rows) = previous {
            for row in rows {
                touched_items.insert(row.item_id.clone());
            }
        }
        let parsed = plugin.parse(root, source, &store.cache_dir())?;
        if parsed.is_empty() {
            changed += 1;
            continue;
        }
        for part in parsed {
            touched_items.insert(part.item_id.clone());
            final_parts.push(NewPart {
                source_key: source.key.clone(),
                item_id: part.item_id,
                mtime_ns: source.mtime_ns,
                size: source.size,
                content_hash: hash.clone(),
                body: serde_json::to_string(&part.body)?,
            });
        }
        changed += 1;
    }

    let planned: HashSet<&str> = sources.iter().map(|s| s.key.as_str()).collect();
    let mut removed = 0u32;
    for (key, rows) in &old_by_key {
        if !planned.contains(key.as_str()) {
            removed += 1;
            for row in rows {
                touched_items.insert(row.item_id.clone());
            }
        }
    }

    let mut by_item: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    for part in &final_parts {
        if touched_items.contains(&part.item_id) {
            by_item
                .entry(part.item_id.clone())
                .or_default()
                .push(serde_json::from_str(&part.body)?);
        }
    }

    let mut upserts = Vec::new();
    for (item_id, mut bodies) in by_item {
        bodies.sort_by_key(std::string::ToString::to_string);
        let assembled = plugin.assemble(crate::plugin::AssembleCtx {
            item_id: &item_id,
            bodies: &bodies,
            media_root: root,
            cache_dir: &store.cache_dir(),
        })?;
        if let Some(jpeg) = &assembled.cover_jpeg {
            std::fs::write(store.cover_path(&item_id), jpeg)?;
        }
        std::fs::write(store.spine_path(&item_id), &assembled.spine_png)?;
        upserts.push(assembled.item);
    }

    let new_ids: HashSet<&str> = final_parts.iter().map(|p| p.item_id.as_str()).collect();
    let mut remove_files = Vec::new();
    for rows in old_by_key.values() {
        for row in rows {
            if !new_ids.contains(row.item_id.as_str()) {
                remove_files.push(row.item_id.clone());
            }
        }
    }
    remove_files.sort();
    remove_files.dedup();

    store.commit_plugin(id, &stamp, &final_parts, &upserts, &remove_files)?;
    Ok(finish(
        id,
        changed,
        unchanged,
        removed,
        started,
        false,
        "reconciled",
    ))
}

fn new_from_old(row: &PartRow) -> NewPart {
    NewPart {
        source_key: row.source_key.clone(),
        item_id: row.item_id.clone(),
        mtime_ns: row.mtime_ns,
        size: row.size,
        content_hash: row.content_hash.clone(),
        body: row.body.clone(),
    }
}

fn finish(
    id: PluginId,
    changed: u32,
    unchanged: u32,
    removed: u32,
    started: Instant,
    skipped: bool,
    reason: &str,
) -> ScanReport {
    ScanReport {
        plugin: id.as_str().to_string(),
        changed,
        unchanged,
        removed,
        elapsed_ms: started.elapsed().as_millis(),
        skipped,
        reason: reason.to_string(),
    }
}

pub fn stamp_of(sources: &[crate::plugin::SourceStat]) -> String {
    let mut lines: Vec<String> = sources
        .iter()
        .map(|s| format!("{}\t{}\t{}", s.key, s.mtime_ns, s.size))
        .collect();
    lines.sort();
    let mut hasher = Sha256::new();
    for line in lines {
        hasher.update(line.as_bytes());
        hasher.update(b"\n");
    }
    hex::encode(hasher.finalize())
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| {
            let ns = d.as_secs() as i128 * 1_000_000_000 + i128::from(d.subsec_nanos());
            ns.min(i64::MAX as i128) as i64
        })
        .unwrap_or(0)
}

pub fn rel_key(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for c in rel.components() {
        match c {
            std::path::Component::Normal(s) => parts.push(s.to_string_lossy().to_string()),
            _ => return None,
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

pub fn jail(root: &Path, rel: &str) -> Option<std::path::PathBuf> {
    if rel.is_empty() || rel.contains('\0') {
        return None;
    }
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return None;
    }
    if rel_path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    let root_c = root.canonicalize().ok()?;
    let file_c = root.join(rel_path).canonicalize().ok()?;
    if file_c.starts_with(&root_c) {
        Some(file_c)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn jail_rejects_escape_and_accepts_a_file_inside() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("cover.jpg"), b"jpeg").unwrap();
        assert!(jail(root, "cover.jpg").is_some());
        assert!(jail(root, "../secret").is_none());
        assert!(jail(root, "/etc/passwd").is_none());
        assert!(jail(root, "nested/../../cover.jpg").is_none());
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret");
        fs::write(&secret, b"no").unwrap();
        std::os::unix::fs::symlink(&secret, root.join("link")).unwrap();
        assert!(jail(root, "link").is_none());
    }
}
