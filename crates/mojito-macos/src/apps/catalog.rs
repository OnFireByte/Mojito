//! Installed-app scan for the settings picker.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use objc2_foundation::{NSBundle, NSString};

#[derive(Clone, Debug)]
pub struct CatalogApp {
    pub name: String,
    pub bundle_id: String,
}

/// Scan on a worker thread; the result lands once through `sender`. The
/// settings window drains it in its poll.
pub fn scan(sender: Sender<Vec<CatalogApp>>) {
    std::thread::Builder::new()
        .name("mojito-catalog".into())
        .spawn(move || sender.send(scan_now()).ok())
        .expect("spawn catalog scan");
}

fn roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }
    roots
}

/// One level deep per root, deduped by bundle id, sorted by name.
fn scan_now() -> Vec<CatalogApp> {
    let mut by_id: BTreeMap<String, CatalogApp> = BTreeMap::new();
    for root in roots() {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "app")
                && let Some(app) = read_bundle(&path)
            {
                by_id.entry(app.bundle_id.clone()).or_insert(app);
            }
        }
    }
    let mut apps: Vec<CatalogApp> = by_id.into_values().collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

fn read_bundle(path: &Path) -> Option<CatalogApp> {
    let bundle = NSBundle::bundleWithPath(&NSString::from_str(&path.to_string_lossy()))?;
    let bundle_id = bundle.bundleIdentifier()?.to_string();
    let name = ["CFBundleDisplayName", "CFBundleName"]
        .into_iter()
        .find_map(|key| {
            bundle
                .objectForInfoDictionaryKey(&NSString::from_str(key))
                .and_then(|any| any.downcast_ref::<NSString>().map(ToString::to_string))
        })
        .unwrap_or_else(|| {
            path.file_stem().map_or_else(
                || bundle_id.clone(),
                |stem| stem.to_string_lossy().into_owned(),
            )
        });
    Some(CatalogApp { name, bundle_id })
}
