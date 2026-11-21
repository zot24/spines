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

pub use config::{choose_music, Config};
pub use domain::{Item, ItemBody, ItemOut, NowPlaying, PluginId, ScanReport};
pub use error::{Error, Result};
pub use plugin::Plugin;
pub use reconcile::reconcile;
pub use store::Store;

pub fn open(cfg: &Config) -> Result<Store> {
    Store::open(&cfg.data_dir)
}

pub fn scan(store: &Store, cfg: &Config, force: bool) -> Result<Vec<ScanReport>> {
    let mut reports = Vec::new();
    for plugin in plugin::registry() {
        let report = match plugin.id() {
            PluginId::Music => match store.music_root()? {
                Some(path) => reconcile(store, plugin.as_ref(), Some(path.as_path()), force)?,
                None => ScanReport {
                    plugin: "music".to_string(),
                    changed: 0,
                    unchanged: 0,
                    removed: 0,
                    elapsed_ms: 0,
                    skipped: true,
                    reason: "unset".to_string(),
                },
            },
            PluginId::Books => reconcile(store, plugin.as_ref(), cfg.books.as_deref(), force)?,
        };
        eprintln!("{}", report.log_line());
        reports.push(report);
    }
    Ok(reports)
}

pub fn shelf_configured(store: &Store, cfg: &Config, plugin: PluginId) -> bool {
    match plugin {
        PluginId::Music => store.music_root().ok().flatten().is_some(),
        PluginId::Books => cfg.books.is_some(),
    }
}
