mod books;
mod bookstyle;
mod config;
mod domain;
mod error;
pub mod http;
pub mod mcp;
mod music;
mod palette;
mod plugin;
mod reconcile;
mod spineimg;
mod store;

#[cfg(test)]
mod tests;

pub use config::Config;
pub use domain::{Item, ItemBody, ItemOut, NowPlaying, PluginId, ScanReport};
pub use error::{Error, Result};
pub use plugin::Plugin;
pub use reconcile::reconcile;
pub use store::Store;

use std::path::Path;

pub fn open(cfg: &Config) -> Result<Store> {
    Store::open(&cfg.data_dir)
}

pub fn scan(store: &Store, cfg: &Config, force: bool) -> Result<Vec<ScanReport>> {
    let mut reports = Vec::new();
    for plugin in plugin::registry() {
        let root = match plugin.id() {
            PluginId::Music => cfg.music.as_deref(),
            PluginId::Books => cfg.books.as_deref(),
        };
        let report = reconcile(store, plugin.as_ref(), root, force)?;
        eprintln!("{}", report.log_line());
        reports.push(report);
    }
    Ok(reports)
}

pub fn media_root(cfg: &Config, plugin: PluginId) -> Option<&Path> {
    match plugin {
        PluginId::Music => cfg.music.as_deref(),
        PluginId::Books => cfg.books.as_deref(),
    }
}
