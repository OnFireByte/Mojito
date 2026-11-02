//! Installed-app scan for the settings picker. This is macOS contact that
//! belongs in mojito-macos long-term; until that crate grows a catalog
//! module, the smallest version lives here (see the slice report).

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

pub fn search<'a>(apps: &'a [CatalogApp], query: &str) -> Vec<&'a CatalogApp> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return apps.iter().collect();
    }
    let mut hits: Vec<(MatchRank, &CatalogApp)> = apps
        .iter()
        .filter_map(|app| rank(app, &query).map(|r| (r, app)))
        .collect();
    hits.sort_by_key(|(r, _)| *r);
    hits.into_iter().map(|(_, app)| app).collect()
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MatchRank {
    NamePrefix,
    WordPrefix,
    NameContains,
    BundleIdContains,
}

fn rank(app: &CatalogApp, query: &str) -> Option<MatchRank> {
    let name = app.name.to_lowercase();
    if name.starts_with(query) {
        Some(MatchRank::NamePrefix)
    } else if name
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word.starts_with(query))
    {
        Some(MatchRank::WordPrefix)
    } else if name.contains(query) {
        Some(MatchRank::NameContains)
    } else if app.bundle_id.to_lowercase().contains(query) {
        Some(MatchRank::BundleIdContains)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, bundle_id: &str) -> CatalogApp {
        CatalogApp {
            name: name.into(),
            bundle_id: bundle_id.into(),
        }
    }

    fn names(apps: &[CatalogApp], query: &str) -> Vec<String> {
        search(apps, query)
            .into_iter()
            .map(|a| a.name.clone())
            .collect()
    }

    fn catalog() -> Vec<CatalogApp> {
        vec![
            app("Android Studio", "com.google.android.studio"),
            app("App Store", "com.apple.AppStore"),
            app("Ghostty", "com.mitchellh.ghostty"),
            app("Slack", "com.tinyspeck.slackmacgap"),
            app("Visual Studio Code", "com.microsoft.VSCode"),
            app("Zen", "app.zen-browser.zen"),
        ]
    }

    #[test]
    fn empty_or_blank_query_keeps_every_app_in_order() {
        let apps = catalog();
        assert_eq!(names(&apps, ""), names(&apps, "   "));
        assert_eq!(names(&apps, "").len(), apps.len());
    }

    #[test]
    fn name_prefix_beats_word_prefix_beats_substring_beats_bundle_id() {
        let apps = vec![
            app("Bistudio", "com.example.b"),
            app("Visual Studio Code", "com.microsoft.VSCode"),
            app("Studio One", "com.presonus.one"),
            app("Recorder", "com.example.studio"),
        ];
        assert_eq!(
            names(&apps, "studio"),
            ["Studio One", "Visual Studio Code", "Bistudio", "Recorder"]
        );
    }

    #[test]
    fn matching_ignores_case_and_surrounding_space() {
        assert_eq!(names(&catalog(), "  GHOST "), ["Ghostty"]);
    }

    #[test]
    fn bundle_id_finds_apps_whose_name_differs() {
        assert_eq!(names(&catalog(), "vscode"), ["Visual Studio Code"]);
        assert_eq!(names(&catalog(), "tinyspeck"), ["Slack"]);
    }

    #[test]
    fn no_match_returns_nothing() {
        assert!(search(&catalog(), "photoshop").is_empty());
    }
}
