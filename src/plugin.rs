use std::path::Path;

use crate::domain::{Item, PluginId};
use crate::error::Result;

/// A media type. The reconciler owns fingerprints and the catalog.
/// A plugin lists files, reads one changed file, and folds those reads into an item.
///
/// Adding a shelf means one type that implements this trait and one line in [`registry`].
pub trait Plugin: Send + Sync {
    fn id(&self) -> PluginId;
    /// Stat the library. Do not read file contents. [`Plan::Absent`] keeps the catalog.
    fn plan(&self, root: &Path) -> Result<Plan>;
    /// Read one source whose bytes changed. `cache` is a writable directory in the data folder.
    fn parse(&self, root: &Path, source: &SourceStat, cache: &Path) -> Result<Vec<ParsedPart>>;
    /// Fold every part that shares an item id into one catalog row and a spine image.
    fn assemble(&self, ctx: AssembleCtx<'_>) -> Result<Assembled>;
}

#[derive(Debug, Clone)]
pub struct SourceStat {
    pub key: String,
    pub abs: std::path::PathBuf,
    pub mtime_ns: i64,
    pub size: u64,
}

#[derive(Debug)]
pub enum Plan {
    /// The library is not there. Leave indexed items alone.
    Absent,
    /// The list is authoritative, including when it is empty.
    Sources(Vec<SourceStat>),
}

#[derive(Debug, Clone)]
pub struct ParsedPart {
    pub item_id: String,
    pub body: serde_json::Value,
}

pub struct AssembleCtx<'a> {
    pub item_id: &'a str,
    pub bodies: &'a [serde_json::Value],
    pub media_root: &'a Path,
    pub cache_dir: &'a Path,
}

pub struct Assembled {
    pub item: Item,
    pub cover_jpeg: Option<Vec<u8>>,
    pub spine_png: Vec<u8>,
}

pub fn registry() -> Vec<Box<dyn Plugin>> {
    vec![Box::new(crate::music::Music), Box::new(crate::books::Books)]
}
