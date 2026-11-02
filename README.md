# Mojito

Mojito is a macOS shortcut app built around one idea: the Caps Lock key becomes the core of machine control, so you accomplish tasks with less finger movement.

> **Status:** early-stage, actively developed. macOS 15.0+ only.

## How it works

A **tap** of Caps Lock sends `Escape`. **Holding** it turns Caps Lock into a Meta key that combines with other keys:

| Shortcut | Action |
| --- | --- |
| Meta + `H` / `J` / `K` / `L` | Arrow keys |
| Meta + `I` | Backspace |
| Meta + `Enter` | Cycle between your two most recent input sources |
| Meta + `E` | Open the app-switching overlay |

Shortcuts are matched by **physical key position**, so they stay consistent no matter what keyboard layout or input language is active.

## App switching

Meta + `E` opens an overlay for switching apps:

- Configure which apps appear in the overlay via Mojito's settings.
- Each app is selected with an assigned label key.
- Pressing `E` again while holding Meta + `E` switches to a live list of currently open apps, each auto-assigned a label key. The layout prioritizes easy-to-reach keys, inspired by the Colemak-DH layout.

Switching brings the correct window into focus, even across workspaces.

## Building from source

Requirements:

- macOS 15.0+
- Rust (the workspace uses edition 2024, so Rust 1.85+)
- Accessibility permission for the built app (it watches Caps Lock, synthesizes keys, and brings windows forward)

```sh
cargo build --release          # build the binary
scripts/build-app.sh           # assemble target/app/Mojito.app (ad-hoc signed)
```

`scripts/build-app.sh` bundles the release binary into `Mojito.app` before first launch so the Accessibility grant attaches to the bundle identity (`com.mojito.app`) and survives rebuilds. If a codesigning identity named `Mojito Dev` exists in your keychain, it is used instead of ad-hoc signing — a stable identity keeps the grant valid across rebuilds.

For lint and tests:

```sh
scripts/check                  # fmt + clippy (mojito crate) + tests (mojito-core)
```

## Project layout

- `crates/mojito` — the app binary: settings UI, app-switcher overlay, login item
- `crates/mojito-core` — the key engine: config, tables, tap/meta state machine (pure, tested)
- `crates/mojito-macos` — macOS integration: event taps, HID filtering, key synthesis, input sources, status item
- `vendor/rune` — vendored copy of [Rune](https://github.com/anomalyco/rune), the UI framework (see `vendor/rune/README-mojito.md` for what was changed)

## License

Mojito is MIT-licensed — see [LICENSE](LICENSE).

The vendored Rune framework in `vendor/rune` is MIT-licensed by its author (see `vendor/rune/LICENSE`). Bundled fonts (Inter, IBM Plex Mono) ship with their own licenses in `vendor/rune/themes/opencode/assets/fonts/`.
