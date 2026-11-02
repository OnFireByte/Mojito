pub use mojito_macos::apps::catalog::{CatalogApp, scan};

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
