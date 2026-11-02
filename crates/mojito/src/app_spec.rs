//! AppSpec builders. Fonts and families come from the pinned theme crate so
//! every surface shares one type system.

use rune::{AppSpec, ConfiguredWindowOptions, WindowOptions};

use crate::ui_settings::{self, SettingsState};

pub fn settings_spec() -> AppSpec<SettingsState> {
    AppSpec {
        fonts: rune_theme_opencode::FONTS,
        families: rune_theme_opencode::FAMILIES,
        poll: Some(ui_settings::poll),
        ..AppSpec::new(
            "Mojito Settings",
            ui_settings::SIZE,
            || unreachable!("App::init replaces AppSpec::init"),
            ui_settings::view,
        )
    }
}

pub fn settings_window_options() -> ConfiguredWindowOptions {
    WindowOptions {
        resizable: false,
        min_size: Some(ui_settings::SIZE),
        max_size: Some(ui_settings::SIZE),
        ..Default::default()
    }
    .into()
}
