# rune API guide

Reference for agents writing UI code in this repo. Distilled on 2026-11-01 from a full read of `anomalyco/rune` branch `dev` at rev `092dec2e40436d6835fe25ebc8a994fe67f3c9e4`. File paths point into that checkout. If a snippet drifts from what compiles, re-check against the pinned rev before trusting this doc.

## Pins and layout

- rune has no releases. Depend on the git dep and pin the rev:

```toml
[dependencies]
rune     = { git = "https://github.com/anomalyco/rune", rev = "092dec2e40436d6835fe25ebc8a994fe67f3c9e4" }
rune-kit = { git = "https://github.com/anomalyco/rune", rev = "092dec2e40436d6835fe25ebc8a994fe67f3c9e4" }
```

- Other crates in that repo: `rune-mac` (macOS helpers), `rune-theme-opencode` (theme as data, see `themes/opencode`), `procwatch` (process footprint watcher). Same rev pin for all.
- Edition 2024 required. Default features: `dbus, embedded-fonts, metal-backend, ogg-audio, svg`. On macOS the Metal backend is default with a precompiled `.metallib`.
- The manual is `crates/rune/README.md` (~2.7k lines). `AGENTS.md` at repo root lists rules. `CHANGELOG.md` section `## dev` lists breaking changes since the pin.

## App skeleton

From `examples/hello/src/main.rs:1-31`:

```rust
use rune::tok::{self, motion, type_};
use rune::{AppSpec, Cx, El, col, el, text};

#[derive(Default)]
struct State { on: bool }

fn view(s: &State, _: &mut Cx<State>) -> El {
    let t = tok::theme(true);
    col().w_full().h_full().center().gap(16.0).fill(t.bg)
        .child(text(if s.on { "Lights on" } else { "Lights off" }).font(type_::TITLE).color(t.text))
        .child(el().size(38.0, 22.0).radius(11.0).motion(motion::SNAPPY)
            .on_press(|s: &mut State| s.on = !s.on))
}

fn main() {
    rune::run(AppSpec::new("Hello", (320.0, 200.0), State::default, view));
}
```

- `AppSpec` fields live in `crates/rune/src/tooling.rs:17-63`: `title`, `size`, `min_size`, `init`, `view`, `scenes`, `fonts`, `families`, `setup: Option<fn(&mut S, Waker)>`, `poll: Option<fn(&mut S, f64, &mut Tx) -> bool>`, `on_key: Option<fn(&mut S, &KeyEvent, &mut Tx) -> bool>`. Return `true` from `on_key` to consume.
- Never write a struct literal for `AppSpec`, `Env`, `OverlaySpec`, `TextStyle`, `RunOptions`, `ScrollSpec`, or `Shadow`. Always end with `..AppSpec::new(..)` or `..Default::default()`. Upstream adds fields and breaks full literals (`AGENTS.md:29`).
- Run an example with `cargo run -p overlay --release` (examples are workspace members). Headless snapshot mode exists for example apps: `target/release/overlay --snap out.png --tree tree.json`.
- Call `rune::prewarm_gpu()` first in `main` (`README:620-629`). It builds the Metal pipeline on a thread while AppKit starts. Defer all other setup to `on_ready`.

## App lifecycle and windows

- `rune::run(spec)` for one window. `run_windows(|displays| vec![WindowSpec..])` for several. `run_windows_with(RunOptions { accessory: true }, |displays| ..)` runs with no Dock icon.
- App builder (`crates/rune/src/app.rs`): `.windowless()` starts with no window, `.keep_running()` keeps the process alive when windows close, `.init(closure)`, `.on_ready(|w: &WindowHandle| ..)` runs once after the first frame, `.on_reopen(..)` (Dock click), `.standard_menus()` adds working Edit and Window menus for accessory apps, then `.run()`.
- `AppHandle<S>` is `Clone + Send + Sync`. Call `handle.update(|s: &mut S, tx| ..)` or `handle.update_if(..)` from any thread. It runs on the main thread in call order and commits on the next frame.
- `WindowOptions` lives at `crates/rune/src/windows.rs:72-124`: `transparent`, `borderless`, `click_through`, `always_on_top`, `visible`, `position` (physical px), `physical_size`, `min_size`, `max_size`, `resizable`. Presets: `WindowOptions::overlay(display)` and `WindowOptions::launcher_panel()`.
- `MacWindowOptions` (`windows.rs:163-199`), attached with `.macos(...)`:

```rust
WindowOptions { transparent: true, borderless: true, ..Default::default() }
    .macos(MacWindowOptions::default()
        .nonactivating_panel(true)     // key-taking NSPanel that never activates the app
        .level(25)                     // NSWindow.Level: normal 0, floating 3, status 25
        .collection_behavior(bits)     // raw NSWindow.CollectionBehavior bitmask, replaces the mask
        .shadow(false)                 // rune draws its own shadow; the system one is rectangular
        .corner_radius(16.0)           // continuous squircle clip of the window
        .exclude_from_capture(true))
```

- `hidesOnDeactivate` is not exposed. A launcher closes itself on focus loss instead: `.on_focus(|_, focused, tx| if !focused { tx.close_window() })` (`examples/launcher/src/main.rs:150`).
- Displays come from the `run_windows` factory closure: `&[Display]` with `id`, `name`, `position` and `size` in physical px, `scale`, `primary` (`windows.rs:44-66`). `Display::native_bounds()` returns top-left-origin bounds.
- There is no public cursor-point API. `DisplayTarget::UnderPointer` resolves the display for overlay placement internally. For exact cursor coordinates, call `CGEventGetLocation` yourself through `objc2` and pass an `OverlayPlacement::Rect`.

## Overlays (the app switcher surface)

`App::overlay(spec, state) -> OverlayHandle<T>` (`crates/rune/src/app.rs:245`, `overlay.rs:69-85`) creates a real non-activating NSPanel that joins all Spaces including fullscreen and sits at status level. Pattern from `examples/overlay-lab/src/main.rs:142-166`:

```rust
rune::run_windows_with(RunOptions { accessory: true }, |displays| {
    let mut app = App::new(spec()).windowless();
    let picker = app.overlay(
        OverlaySpec::new("Mojito switcher", (680.0, 160.0), view)
            .options(OverlayOptions {
                display: DisplayTarget::Primary,
                placement: Some(OverlayPlacement::FullScreen),
                level: OverlayLevel::AboveStatus,   // or AboveOverlays(27)
                input: OverlayInput::All,           // None | Regions | All
                keys: KeyPolicy::OnDemand,          // enables handle.request_keys() later
                exclude_from_capture: true,
                prewarm: true,                      // panel built hidden at alpha 0, first show is instant
                ..Default::default()
            })
            .on_key(key_handler),                   // fn(&mut T, &KeyEvent, &mut Tx) -> bool
        state);
    vec![WindowSpec::from_app("lab", app)
        .options(rune::WindowOptions { transparent: true, ..Default::default() })]
});
```

- `OverlayPlacement` (`overlay/options.rs:52-62`): `Edge { edge: ScreenEdge::Top|Bottom, margin, align, inset }`, `Rect { x, y, width, height }` in points from the primary display top-left, or `FullScreen`. `DisplayTarget`: `Primary | UnderPointer | Id(..) | Native(..)`.
- `OverlayHandle` methods (`overlay.rs:140-259`): `show`, `hide`, `fade_in(Duration)`, `fade_out(Duration)`, `set_placement`, `place(edge, margin)`, `set_align`, `set_display`, `resize`, `set_macos_level(isize)`, `set_input`, `request_keys`, `set_exclude_from_capture`, `window_number() -> Option<u32>`, `update(|s, tx| ..)`, `update_if`, `availability()`.
- Keyboard focus in the overlay without activating the app works only with `keys: KeyPolicy::OnDemand` plus `request_keys()`. Interactive overlays are unavailable on Wayland.
- Mouse hover does not reach a non-key panel on macOS (`README:1936-1937`). Use `OverlayInput::Regions` and mark hit rects with `.hit()` on fill-less elements.
- Show latency: with `prewarm: true`, call `show()` plus one `update(...)` in the same batch and use `tx.instant()` so the first visible frame already holds the new state (`src/rune_ui/live_text.rs:92-121` in hex does this).

## View model and widgets

- Retained keyed tree rebuilt from plain view functions. Every handler re-runs `view`, rune diffs the tree. Set `.key("...")` on every stateful node so identity survives reordering.
- Constructors re-exported from `rune` root (`lib.rs:59-120`): `el()`, `col()`, `row()`, `text()`, `rich(text, spans)`, `text_input(&TextEdit, style)`, `icon(&Icon, size)`, `spacer()`, `canvas(shape)`.
- Layout: `.w/.h/.w_full/.grow/.gap/.padding/.px/.py/.center/.center_y/.absolute/.pin(l,t,r,b)`. Paint: `.fill/.border/.radius/.clip_rounded/.shadow/.box_shadow/.gradient/.opacity/.blur/.backdrop_blur/.system_material`. Input: `.on_press/.on_press_tx/.on_hover/.on_drag/.on_key/.on_input_key/.on_physical_input/.on_wheel/.on_secondary_press/.on_double_press`. Focus: `.focusable(true)/.focus_group()`. Also `.scroll(ScrollSpec)` and `.hit()` (required on fill-less clickable elements).
- `rune-kit` components (`crates/rune-kit/src/lib.rs:58-102`), built as `Component::new(..).build(&k)`: `Button, IconButton, Badge, Card, Checkbox, Switch, Radio, Segmented, Combobox, Select, Dialog, Menu, List/ListItem, TextInput, TextArea, Table, Tree, Accordion, Collapsible, Toast, Progress, Spinner, Markdown, Keybind, ShortcutRecorder`.
- Flexbox traps (`README:20-32`): `.grow()` is flex-basis 0, so ancestor heights must be bound. Nested flex columns need `.min_h(0.0)` or inner scrolls never scroll. `.virtualize` requires `.growth(GrowthSpec)` on a growth stack.
- `TextStyle` is a plain struct. Use the `tok::type_` constants or `TextStyle::new(size, weight)`.

## Theming and the liquid glass look

- Tokens in `rune::tok` (`crates/rune/src/tok.rs`): `tok::theme(dark) -> Theme` with `bg, card, card_border, text, text_secondary, separator, control_track, control_off, hover, press, shadow`, and more. `tok::accent(i, dark)` gives six OKLCH accents. `tok::type_::{BODY, LABEL, SECTION, TITLE, MONO_CAPTION}`. `tok::space::{S1=4,S2=8,S3=12,S6=24}`. `tok::radius::{SM=6,MD=8,LG=12}`. `tok::motion::{SNAPPY, DEFAULT, CALM, LEAD}`. Colors: `oklch(l,c,h)`, `oklcha(.., alpha)`, `Color::WHITE.alpha(0.1)`.
- A full theme as data: `themes/opencode` exposes `theme(dark) -> &Theme`, `kit(dark) -> &Kit`, and `FONTS`/`FAMILIES` for `AppSpec::fonts/families`. Hex passes these straight into its settings window.
- The Apple liquid glass idiom, from `examples/glass-panel/src/main.rs:161-168`:

```rust
el().size(w, h).radius(radius).continuous_corners()
    .box_shadow(Shadow::core_animation(0.0, 0.5, 1.5, BLACK.alpha(0.3)))    // contact shadow
    .box_shadow(Shadow::core_animation(0.0, 8.0, 18.0, BLACK.alpha(0.32)))  // ambient shadow
    .system_material(SystemMaterial::Glass)   // NSGlassEffectView on macOS 26+, Popover fallback below
    .child(el().radius(radius).clip_rounded(radius)
        .fill(ink_bg.alpha(0.62))             // frost tint
        .border(1.0, ink.alpha(0.08))         // hairline
        .child(content))
```

- `SystemMaterial::Glass` blurs behind the window and needs a transparent window. Set `MacWindowOptions::shadow(false)` because rune draws the shadow. Add `.srgb_blend(true)` on the window or overlay for AppKit-matching colors. `Shadow::core_animation(x, y, radius, color)` takes a CALayer radius and sets blur to twice the radius (`README:2145-2157`).
- In-window frosted panels over your own content: `.backdrop_blur(8.0).fill(tint)` (`examples/frosted-bar/src/main.rs:65-68`). At most 4 backdrop blurs per frame.

## Input and key capture

- Routing order (`README:210-258`): global shortcuts and native menus, then IME, default button, window `on_key` when nothing is focused, focused field `on_input_key`, focus navigation, text editor, `on_key` again, selection keys, scroll paging.
- `KeyEvent { key: Key, mods: Mods }`. `Key` variants (`edit/keys.rs:18-53`): `Text(String), Enter, Escape, Tab, Left/Right/Up/Down, Home/End, PageUp/Down, Backspace, Delete, Command(char), Function(u8), ContextMenu, Modifier{..}`. `tx.physical_key()` gives the layout-independent key, which is what trigger-key assignment needs.
- Raw physical keys on any element: `.on_physical_input(|s, &PhysicalInput::Key{key,mods,pressed,repeat} | PhysicalInput::Modifiers(m) | PhysicalInput::FocusLost, tx| -> bool)`. `FocusLost` also fires when the window deactivates (`input/physical_input.rs:5-17`).
- Text input: hold a `TextEdit` in state (`TextEdit::new("")`, set `.focused = true`), then `text_input(&s.edit, type_::BODY.lh(26.0)).single_line().placeholder(..).on_edit(..).on_input_key(..).on_submit(..)` (`examples/launcher/src/main.rs:78-92`).
- For assigning a chord in settings, `rune-kit` has `ShortcutRecorder` (`crates/rune-kit/src/shortcut_recorder.rs`): captures modifiers plus physical key, Esc cancels, Backspace clears, `on_change` hands you a `Shortcut` to register or reject with `Err(ShortcutError)`.
- Global hotkeys exist (`crates/rune/src/shortcuts/`): `App::global_shortcut("cmd+space", |s, tx| ..)` at build time or `handle.register_shortcut("ctrl+alt+k".parse::<Shortcut>()?, handler)` at runtime. macOS backend is Carbon `RegisterEventHotKey` and needs no permission. Limits: chord press only, no down/up distinction, no tap-vs-hold, Caps Lock cannot register. Mojito's engine therefore lives outside rune.

## Threads, timers, high-frequency values

- Background threads talk to the UI only through `AppHandle::update` / `OverlayHandle::update` and the `Live`/`Latest`/`Pixels` values. Nothing else is thread-safe.
- `AppSpec.setup(Some(|s: &mut S, waker: Waker| ..))` hands you a waker. A producer thread calls `waker.wake()` and `poll` drains the state change. Returning `false` from `poll` keeps an idle window at zero draws.
- `rune::Latest<T>` for latest-wins background work (search as you type), answered in `poll` (`README:730-766`). `rune::Live<[f32;2]>` bound with `.live_offset(&s.value)` moves things without re-running the view (`README:1106-1121`). `tx.wake_after(Duration)` schedules a timer tick that still fires while hidden.
- `Tx::instant()` skips spring animation for one change.

## What rune does not do (drop to objc2 / CoreGraphics)

- Event taps, key interception, key synthesis. Rune has no API for this.
- A macOS menu bar status item. `rune::Tray` is Linux and Windows only (`tray.rs:1-9`). Build `NSStatusItem` with `objc2`; hex shows the pattern in `crates`-external code at hex `src/macos_rune/status_item.rs:46-120`.
- `hidesOnDeactivate`, per-window activation policy beyond the accessory toggle, and collection-behavior sugar beyond the raw bitmask.
- A mouse cursor over the app when it is not frontmost, unless you enable the `macos-background-cursor` feature, which uses private API (`overlay/options.rs:100-103`).
- Packaging: a bare `cargo run` binary has no bundle id, so TCC permissions attach to nothing. Use `scripts/mac-app --name Mojito --id com.<you>.mojito --exe mojito --bin target/release/mojito` from the rune checkout (`crates/rune/README.md:775-793`), or copy that script's approach into this repo.
- No AccessKit accessibility tree yet (`README:2711-2720`). Vertical scrolling only.

## Mojito notes

- The switcher overlay should be `App::overlay` with `prewarm: true`, `input: OverlayInput::Regions` (or `All` if we want real mouse use), `keys: KeyPolicy::OnDemand` plus `request_keys()` while visible, and a glass card built per the section above.
- The settings surface is a regular window opened from a status item action through `handle.open()` / `App::window`, not an overlay.
- The key engine (CGEventTap) runs on its own thread and drives the UI only via `OverlayHandle::update` / `AppHandle::update` and `show`/`hide`.
