# Mojito architecture

The synthesized design for Mojito, produced by an architect arena (three candidates, one cross-judge; provenance in the Synthesis section at the end). This document is the contract for implementation. Data shapes come first; every module below implements against them.

## Goal

Mojito turns Caps Lock into the hub of machine control so the user reaches any target with fewer finger movements. Tap Caps Lock sends Esc. Holding it engages a meta layer: meta+h/j/k/l are arrow keys, meta+i is backspace, meta+enter cycles input languages, meta+m opens the app switcher. Bindings are physical key positions and hold under any layout or input language. The switcher lists user-pinned apps with user-assigned trigger keys; pressing m again toggles to open apps with auto-assigned easy-reach labels (Colemak-DH ergonomics). Switching changes the window, not just the app. A settings window manages pinned apps and keys. macOS 15+ first, Rust, rune UI, resource budget: about zero idle CPU and an instant overlay.

## Shape in one paragraph

Two threads. The tap thread owns two CGEventTaps (a HID-head filter and an annotated-session fallback), a health timer, and the entire decision engine, which runs inline in the tap callback as a pure state machine. The main thread runs rune (overlay panel, settings window, status item), NSWorkspace observers, and all AX/TIS/AppKit work drained from a job queue inside `AppSpec.poll`. The engine never blocks and never touches AppKit; the UI never parses a keystroke. Shared state flows one way through lock-free snapshots (`ArcSwap<Config>`, `ArcSwap<AppIndex>`), and the only engine-to-UI vocabulary is two message enums.

```
tap thread                              main thread
┌─────────────────────────────┐   UiMsg  ┌──────────────────────────────┐
│ HID tap (filter, head)      │ ───────► │ rune loop (poll drains jobs) │
│ session tap (filter, fall-  │          │  ├ switcher overlay (prewarm)│
│   back for injected keys)   │          │  ├ settings window           │
│        │                    │   Job    │  ├ NSStatusItem              │
│   Engine::on_event (pure)   │ ───────► │  ├ NSWorkspace observers ─┐  │
│        │  Synth posts inline│          │  └ TIS / AX / activate    │  │
│        ▼                    │          └──────────────────────────┼──┘
│  ArcSwap<Config> (read)     │ ◄─── publishes ArcSwap<AppIndex> ───┘
└─────────────────────────────┘
```

## Crates

| crate | owns | depends on |
|---|---|---|
| `mojito-core` | Pure domain: key newtypes, engine state machine, binding tables, label assignment, config model and JSON parsing, overlay placement math, glyph table. No macOS, no rune, no I/O. Everything here runs in `cargo test` on any OS. | serde, arc-swap |
| `mojito-macos` | All macOS contact: taps, synthesis, hidutil remap, app index, switching, TIS, permissions, status item, layout glyphs. `cfg(target_os = "macos")`. | core, objc2, objc2-foundation, objc2-app-kit |
| `mojito` (bin) | Wiring only: rune app assembly, UI views, the bridge from `UiMsg`/`Job` to rune handles, packaging entry. | core, macos, rune, rune-kit |

Dependency direction: `mojito → {mojito-macos, mojito-core}`; `mojito-macos → mojito-core`; `mojito-core → nothing`. `UiMsg` and `Job` are the only engine/UI crossings. `ModifierFlags` is opaque in core; no CoreGraphics or AX type appears outside `mojito-macos`.

## Constants (verified, load-bearing)

From the macOS SDK `HIToolbox/Events.h` and the USB HID usage tables. The golden test pins these; the arena's three candidates each had F18 wrong.

```rust
/// CGEvent virtual keycode = physical position, layout-independent.
pub struct PhysicalKey(u16);
/// HID usage in the keyboard page (hidutil world). A different numbering
/// space from PhysicalKey; the newtypes make a mix-up not compile.
pub struct HidUsage(u32);

pub const CAPS:     PhysicalKey = PhysicalKey(0x39); // kVK_CapsLock = 57
pub const F18:      PhysicalKey = PhysicalKey(0x4F); // kVK_F18 = 79 (SDK Events.h)
pub const CONTROL:  PhysicalKey = PhysicalKey(0x44); // unassigned control keycode
pub const CAPS_USAGE: HidUsage   = HidUsage(0x700000039);
pub const F18_USAGE:  HidUsage   = HidUsage(0x70000006D); // 30064771181, TN2450 encoding

// Positions the engine and labels key off (virtual keycodes):
// h=0x04 j=0x26 k=0x28 l=0x25 i=0x22 m=0x2E esc=0x35 return=0x24
// backspace kVK_Delete=0x33; arrows Left=0x7B Right=0x7C Down=0x7D Up=0x7E
```

## Core types

```rust
pub struct RawKey { pub key: PhysicalKey, pub down: bool, pub repeat: bool,
                    pub flags: ModifierFlags, pub t: Mono } // t = monotonic at callback entry

/// The engine's whole output. Pass/Swallow decide the event's fate; the rest execute.
pub enum Action {
    Pass,                       // hand the event to macOS untouched
    Swallow,                    // return NULL from the tap
    Synth(SynthKey),            // post a marked event at kCGHIDEventTap, inline
    Ui(UiMsg),                  // engine -> overlay, via channel + waker
    Job(Job),                   // engine -> main thread, via channel + waker
}
/// Down and up are explicit. `flags` carries the triggering event's real modifier
/// flags, so meta+shift+h synthesizes shift+Left (selection preserved).
pub struct SynthKey { pub key: PhysicalKey, pub down: bool, pub flags: ModifierFlags }

pub struct Engine {
    phase: Phase,
    config: ArcSwap<Config>,     // swapped by settings save; version-checked
    tables: Tables,              // rebuilt lazily when config.version changes
    index:  ArcSwap<AppIndex>,   // published by main thread
    route:  [Route; 256],        // per-hold decision memory: down decides, up obeys
    memory: LabelMemory,         // sticky labels, merged to disk (see labels)
    ui: Sender<UiMsg>, jobs: Sender<Job>,
}

/// One F18 press lifecycle. Meta engages eagerly on F18 down (no threshold lag);
/// Esc is decided at release; no run-loop timers exist anywhere in the engine.
enum Phase {
    Idle,
    Armed { down_at: Mono, used: bool },
    /// Meta layer active, overlay hidden: bound chords route, unbound keys
    /// stay silent. `committed` makes the rest of the hold inert after a
    /// switch so leftover autorepeats never type into the app we landed on.
    Held { committed: bool },
    /// The switcher is visible and survives meta release; while it shows,
    /// every bare key is ours. F18 may be up or down.
    Overlay { overlay: OverlayPhase, committed: bool },
}
enum OverlayPhase { Hidden, Pinned, Live(Box<LiveModel>) }
```
An engine field `meta_down` tracks whether F18 is physically down, because commit and cancel land differently: with meta still held they return to the chord layer (inert when committed), without it they return to transparent Idle.
enum Route { Undecided, Pass, Swallow }

/// Invariants of Engine::on_event (pure; no I/O; never blocks; O(1) per event):
/// - F18 is always swallowed. Real caps behavior can never leak (the hidutil
///   remap guarantees macOS never sees a Caps Lock event at all).
/// - Any key event during the hold (including passed modifier chords) sets
///   `used = true`, which kills the Esc-on-release. A bare modifier press
///   (flagsChanged) is invisible to the key tap and does not count.
/// - Keys with CMD/CTRL/OPT flags pass through untouched (cmd+tab survives a
///   meta hold); their down-route is remembered so the up passes too.
/// - SHIFT-only chords are honored: meta+shift+h synthesizes shift+Left.
/// - Autorepeat of a routed key re-synthesizes (held h = held Left).
/// - An event identical to the previous one (same key, direction, timestamp)
///   repeats the previous decision with no side effects: the HID-head tap
///   and the session fallback both see one hardware event, and only the
///   first delivery may act.
/// - Any reset (tap invalidated, sleep, health-timer fire) returns to Idle,
///   clears `route`, and hides the overlay.
pub fn on_event(&mut self, ev: RawKey) -> Vec<Action>;
```

### Binding tables

The whole domain is two 128-slot tables built by one pure function; no runtime branching decides what a key means.

```rust
pub struct Tables { hidden: [Option<Chord>; 128], pinned: [Option<Chord>; 128] }
pub enum Chord {
    Synth(SynthKey),     // arrows/backspace; flags ride along
    CycleInput,          // fresh presses only, autorepeat ignored
    OpenPinned,          // meta+m
    ToggleMode,          // m again, both directions
    TriggerPin(u16),     // index into Config::pinned
    Label(u8),           // index into LiveModel rows
    Cancel,              // esc inside the overlay
}
// hidden[m]: h/j/k/l -> Synth(arrows), i -> Synth(backspace), enter -> CycleInput,
//            m -> OpenPinned, everything else None (pass iff flags has
//            CMD/CTRL/OPT, else swallow).
// pinned[m]: user trigger keys -> TriggerPin, m -> ToggleMode, esc -> Cancel.
//            Labels are never in pinned[m]; live labels come from LiveModel.
```

### Live model and labels

```rust
/// Frozen when Live is entered; never mutated during the hold. Labels cannot
/// reshuffle under the user's fingers.
pub struct LiveModel { pub rows: Arc<[LiveRow]> }
pub struct LiveRow { pub pid: i32, pub bundle: Option<String>, pub name: String,
                     pub windows: u8, pub label: Option<PhysicalKey> }
```

Label priority encodes the Colemak-DH ergonomics (grounded against colemakmods.github.io/mod-dh): the ten easiest keys are the eight outer home-row positions plus the two bottom-row index-curl positions where the mod places D and H; the center columns (qwerty G/H positions) are the positions the mod exists to demote, so they rank below the strong top-row keys. Order within a tier is a product knob; tests pin tier membership and determinism, not intra-tier order.

```rust
/// QWERTY-position virtual keycodes, best reach first. First ten = the
/// Colemak-DH "ten easiest keys" (home row outer eight + bottom index curl).
pub const LABEL_PRIORITY: [PhysicalKey; 27] = [
    /* home L index->pinky */ F, D, S, A,
    /* home R index->pinky */ J, K, L, SEMICOLON,
    /* bottom index curl   */ V, M,
    /* top strong          */ T, Y, E, U, R, I,
    /* home center (demoted)*/ G, H,
    /* bottom neighbors    */ B, N,
    /* bottom left         */ C, X, Z,
    /* top pinky/ring      */ Q, W, O, P,
];
```

Assignment is deterministic and injective: walk `LABEL_PRIORITY`, skipping every key in `RESERVED` and every user-pinned trigger key; hand keys to rows in NSWorkspace recency order; then apply sticky memory (an app keeps its last label when that key is free this activation). Rows beyond the pool render unlabeled and are unreachable by key (rare; more than ~24 open apps).

```rust
/// Keys no label or pin may ever take. Enforced in config validation AND in
/// label assignment; single source of truth.
pub const RESERVED: [PhysicalKey; 4] = [F18, ESC, RETURN, M];
pub struct LabelMemory(BTreeMap<String /*bundle_id*/, PhysicalKey>);
pub fn assign_labels(rows: &[AppSlot], memory: &LabelMemory, pinned: &[PhysicalKey])
    -> Vec<LiveRow>;
```

Deliberate simplifications: no h/j/k/l cursor navigation inside the overlay (label keys are the whole selection model, per goal; nav is a fast-follow), and the frontmost app is excluded from the Live list (switching to yourself is a no-op).

### Config and persistence

```rust
pub struct Config { pub pinned: Vec<PinnedApp>, pub tap_max_ms: u16 /* 250 */,
                    pub capture_enabled: bool, pub version: u64 }
pub struct PinnedApp { pub name: String, pub bundle_id: String, pub key: PhysicalKey }
```

- File: `~/Library/Application Support/Mojito/settings.json`, serde JSON, pretty, written through a durable writer (fsync file and directory, one mutex).
- Recovery rules copied from hex (`app_settings/persistence.rs:38-179`): unknown keys ignored; an invalid field defaults in memory with a per-key warning; an unreadable file loads defaults and leaves the original untouched; before overwriting an invalid file, preserve `settings.json.unreadable-<sha256>`; duplicate trigger keys resolve first-wins with a warning; reserved keys rejected with a warning.
- Ownership: the settings window is the only writer. Save = validate at the boundary, durable write, `ArcSwap::store`. The engine reads `Config` per event and rebuilds tables when `version` changes, so edits apply on the next gesture with no restart and no file watcher (the app is the only writer).
- Uniqueness by construction is preferred where it is free: keep `Vec<PinnedApp>` for display order, and reject duplicate keys at parse and at capture.

### Bridge messages

```rust
/// Full small snapshots, not diffs; the overlay replaces state wholesale.
pub enum UiMsg {
    ShowPinned { rows: Arc<[PinnedRow]> },
    ShowLive   { rows: Arc<[LiveRow]> },
    Hide, HideFade,        // instant hide on commit, short fade on cancel
}
pub struct PinnedRow { pub key: PhysicalKey, pub name: String, pub glyph: char }

pub enum Job {
    Switch(SwitchRequest),        // activate + AX raise, on main
    CycleInputSource,             // TISSelectInputSource, on main (TIS is main-thread-only)
    RefreshWindowCounts,          // CGWindowList counts into AppIndex, on main
    MergeLabels(LabelMemoryPatch) // sticky labels -> disk, debounced
}
pub enum AppRef { Pid(i32), Bundle(String), Launch(PathBuf, String) }
pub struct SwitchRequest { pub app: AppRef }
```

## Engine decision table

The whole UX in one matrix. `route[key]` makes every down decision bind its up; autorepeats follow their down.

| key (physical) | Hidden (meta held) | Pinned | Live |
|---|---|---|---|
| h/j/k/l | synth arrow (flags kept) | label or swallow | label or swallow |
| i | synth backspace | label or swallow | label or swallow |
| return | CycleInput (fresh only) | label or swallow | label or swallow |
| m | OpenPinned | ToggleMode | ToggleMode |
| user trigger key | (unbound: swallow) | TriggerPin -> commit | n/a |
| assigned label key | n/a | n/a | Label -> commit |
| esc | swallow | Cancel (HideFade) | Cancel (HideFade) |
| any key, flags CMD/CTRL/OPT | Pass (and marks used) | Pass (and marks used) | Pass (and marks used) |
| any other bare key | swallow | swallow | swallow |
| F18 up | tap within tap_max_ms and !used -> synth Esc; else nothing -> Idle | overlay stays open | overlay stays open |

Commit = `Ui(Hide)` + `Job(Switch)` + `committed = true` while meta is held, or an immediate return to transparent Idle when the overlay was already sticky. Esc cancels with `HideFade`. The overlay is sticky: releasing meta leaves it open, and while it shows the engine owns every bare key; Esc, a selection, or the control keycode closes it. Click-to-cancel reaches the engine because the UI posts an unmarked synthetic keyDown on the reserved unassigned `CONTROL` keycode, which the tap swallows and turns into a cancel.

## macOS backend

- **Taps** (`mojito-macos::taps`): two filter taps on one CFRunLoop thread sharing one callback. The key tap sits at `kCGHIDEventTap` with head insertion, mask = keyDown | keyUp: the head of the HID chain, because another app's HID-level key filter (iTerm2's hotkey window) otherwise consumes keys before any session tap sees them. The fallback tap sits at `kCGAnnotatedSessionEventTap`, same mask and callback: it catches keys injected above the HID level that no hardware produced (Touch-Bar-style Esc, driver-synthesized keys). One hardware event therefore crosses both taps when passed; the engine deduplicates by event identity (key, direction, timestamp) and repeats the first decision without side effects. flagsChanged is never filtered (an active filtering tap there breaks Finder's Option menus even on pass-through). Callbacks never block: pure `on_event` inline, `Synth` posted inline, `Ui`/`Job` over unbounded channels with `try_send` plus a waker poke. A 5 s health timer re-enables taps macOS silently disabled and recreates invalidated ones (either tap's gap is a coverage interruption, so the engine resets), then injects `Reset` so the engine returns to Idle and the overlay hides. App Nap opted out on the tap thread only (`UserInitiatedAllowingIdleSystemSleep`). Readiness over a `sync_channel` with a 2 s timeout; the session tap failing to create degrades to HID-only with a log line.
- **Synthesis** (`mojito-macos::synth`): `CGEventCreateKeyboardEvent` + `CGEventSetFlags` + `CGEventSetIntegerValueField(kCGKeyboardEventIntegerFieldEventSourceUserData = 42, MARKER)` + `CGEventPost(kCGHIDEventTap)`. Marker `0x004D_4F4A_3154` ("MOJ1T"); the tap ignores marked events so we never intercept our own output. No chord-breaker posting: F18 is not a modifier, so no system modifier state is ever held and no app can fire a modifier hotkey on meta. The primitive stays available for a future keycode-57 fallback mode.
- **Caps remap** (`mojito-macos::caps`): `hidutil property --get UserKeyMapping`, merge our `{src: 0x70000039, dst: 0x7000006D}` entry preserving foreign entries, `--set` (no root needed). Persist with `~/Library/LaunchAgents/com.mojito.caps.plist` (RunAtLoad, runs the same set), loaded via `launchctl bootstrap gui/$(id -u)`. Self-heal: a periodic `is_active()` check reinstalls and badges the status item if the mapping vanished (Sequoia Login-Items consent can disable the agent). Uninstall removes only our entry, matched on `Src == CAPS_USAGE` (a user may remap something else TO F18), boots out the agent, deletes the plist. Capture-off toggle restores stock Caps Lock.
- **App index** (`mojito-macos::index`): main thread maintains a recency ring of running regular apps from NSWorkspace activation/launch/terminate observations and publishes `ArcSwap<AppIndex>`; `Job::RefreshWindowCounts` adds CGWindowList layer-0 window counts per pid when Pinned mode opens, so they are fresh by the time the user presses m again. The count spans every Space (`kCGWindowListOptionAll` + `kCGWindowListExcludeDesktopElements`, minimized windows included): an on-screen-only count saw one app when the frontmost Space was fullscreen, which emptied the live list. Live list = regular apps with at least one window on any Space, frontmost excluded, recency order. No `kCGWindowName` reads anywhere (it demands Screen Recording permission).
- **Switching** (`mojito-macos::switch`): every AX call under `AXUIElementSetMessagingTimeout(0.25)` so a hung app cannot stall the gesture. Ladder: resolve pid by bundle id (`NSRunningApplication` lookup, else `NSWorkspace.openApplication` then re-resolve) -> `unhide()` if hidden -> `activate()` -> pick window via AX (`kAXFocusedWindowAttribute` -> `kAXMainWindowAttribute` -> windows[0] -> none) -> `kAXRaiseAction`. AXRaise runs even when `activate()` is refused; both run unconditionally. Degradation: Accessibility denied or no raisable window -> activation-only, and the settings pane says so plainly; switching to a Space with open windows completes the move. Minimized targets: clear `kAXMinimized` before raise.
- **Input sources** (`mojito-macos::tis`): enabled sources via `TISCreateInputSourceList(null, false)` filtered to keyboard-category and select-capable; cycle = next of current via `TISSelectInputSource`, on the main thread only (overlapping TIS calls from worker threads crashed hex, release 2.1.12). This replaces the native Caps Lock input cycling the remap removes.
- **Permissions** (`mojito-macos::permission`): `CGPreflightPostEventAccess()` (launch-frozen) + `AXIsProcessTrusted()` (live) both checked; prompt via `AXIsProcessTrustedWithOptions(kAXTrustedCheckOptionPrompt: YES)`; 250 ms poll starts the engine the moment the grant lands and stops once granted; still untrusted ~15 s after prompting offers a `tccutil` reset and re-prompt. Input Monitoring is never requested. Accessibility also grants listen-event access.
- **Status item** (`mojito-macos::status_item`): `NSStatusItem` via objc2 (hex `macos_rune/status_item.rs:46-120` pattern), menu: Settings, Start at Login (SMAppService on a worker thread), Restore Caps Lock, About, Quit. Actions cross to the pump over a `SyncSender`.
- **Layout glyphs** (`mojito-macos::layout`): one main-thread snapshot at startup of `UCKeyTranslate` over keycodes 0..128 from `TISCopyCurrentKeyboardLayoutInputSource`, refreshed on the distributed input-source-changed notification; converts `PhysicalKey` to display glyph for badges. TIS access stays behind one mutex and never re-enters from worker threads on a miss.

## UI (rune, pinned rev 092dec2e40436d6835fe25ebc8a994fe67f3c9e4)

- App assembly: `rune::prewarm_gpu()` first in `main`; `run_windows_with(RunOptions { accessory: true }, ..)` with a `windowless().keep_running()` app; status item and engine started in `on_ready` so nothing the first frame needs waits on them.
- **Switcher overlay**: one prewarmed panel via `App::overlay` (hex `rune_hud.rs:299-353` shape): `OverlayOptions { display: UnderPointer, prewarm: true, input: OverlayInput::Regions, keys: KeyPolicy::None, exclude_from_capture: true }`, `.srgb_blend(true)`. `KeyPolicy::None` is deliberate: the tap owns every keystroke, so the panel is a pure display surface and the focus-loss race class does not exist. Reveal in one `update` closure: cursor position (via `CGEventGetLocation` in `mojito-macos`) -> display containing it -> `OverlayPlacement::Rect` centered (pure math in core) -> store rows -> `tx.instant()` -> `show()`; the first visible frame already holds the rows. `OverlayInput::Regions` plus a `.hit()` backdrop gives click-to-cancel without focus.
- Glass card per `examples/glass-panel`: `continuous_corners()`, two `Shadow::core_animation` shadows, `system_material(SystemMaterial::Glass)`, frost tint fill, hairline border; window shadow off. Rows: key badge (glyph) | app icon or monogram | name | (live) window count.
- **Settings window**: regular window (fixed-size so tiling window managers float it), opened from the status item; activation policy flips Accessory -> Regular while open and back. Panes: pinned apps (add from installed-app catalog scan, assign trigger key, remove), behavior (tap threshold), permissions (live state, open System Settings, degraded-mode banner), remap status (installed/reinstall/restore). Trigger capture happens in the window via `.on_physical_input`: the tap engine passes bare keys through when meta is not held, so the focused settings window receives them; the recorder shows the glyph from the layout snapshot. rune-kit `ShortcutRecorder` is rejected for now: it models modifier chords, and our triggers are bare physical keys.
- Theme: `rune-theme-opencode` tokens (`theme(dark)`, `FONTS`, `FAMILIES`); no color literals outside the theme; one accent per surface; hover instant; settings rows never hover-fill.

## Process shape

- Threads: main (rune AppKit loop, observers, status item, windows, all TIS/AX/activation via jobs) and tap (two taps, health timer, engine inline). Nothing else. Idle CPU about zero: run loops park, hidden prewarmed overlays run handler frames without drawing and pace themselves (rune `overlay/options.rs:179`, `overlay/more_tests.rs:119` — verified in source), `poll` returns false when nothing drained, the permission poll stops after the grant.
- Quit discipline (hex `macos_rune.rs:498-528`): observe `NSApplicationWillTerminateNotification` before the loop runs; on quit, flip the shutdown flag, stop the taps, restore the hidutil mapping if capture is off, join the tap thread bounded, then `_exit(0)`.
- Packaging: `scripts/build-app.sh` mirrors hex's `build-app.sh`: `cargo build --release`, assemble `Mojito.app/Contents/{MacOS,Resources}`, Info.plist with `CFBundleIdentifier`, `CFBundleExecutable`, `LSUIElement=true`, `LSMinimumSystemVersion 15.0`, `NSAccessibilityUsageDescription`; ad-hoc `codesign --force --sign -`. Bundle before first run so the TCC grant attaches to the bundle id. `scripts/check` runs fmt, clippy `-D warnings`, and tests. Files stay under 800 non-test lines.

## Test and verification plan

Pure, `cargo test`, any OS (all in `mojito-core`):
- Engine golden sequences: tap -> exactly one Esc (down+up); hold+h -> Left down; held h autorepeat -> repeated Left downs, one up at release; meta+shift+h -> shift+Left; meta+enter -> CycleInput once; meta+m -> ShowPinned; m again -> ShowLive; m again -> Pinned; label -> commit + committed-inert leftovers; release -> Hide; esc -> HideFade; cmd+tab during hold -> Pass and marks used; unbound bare key -> swallow; F18 up stale (> tap_max_ms) -> no Esc; F18 up after a passed chord -> no Esc; route memory: every up obeys its down's decision, including after release; injected Reset from any state -> Idle + Hide; duplicate event (same key, direction, timestamp) repeats the decision with no side effects.
- Tables: hidden table equals the const spec; pinned table trigger keys win; duplicate trigger keys never yield two TriggerPins; reserved keys never yield chords.
- Labels: determinism, injectivity, RESERVED skipped (m never assigned), pinned keys skipped, sticky memory honored when free and skipped when taken, priority tier membership pinned, overflow -> unlabeled.
- Keycode spaces: golden test asserts `F18 = 0x4F`, `F18_USAGE = 0x7000006D`, and that the two constants differ (the trap that killed all three arena candidates).
- Config: every recovery rule; duplicate keys first-wins; reserved keys rejected; round-trip.
- Placement math: display containing a point across scaled and negative-origin displays; centered rect clamped to visible frame.

Needs a human on macOS: LED stays dark and Caps never reaches any app (TextEdit before/after); Esc tap feels instant in Terminal; arrows and backspace under QWERTY, Dvorak, and a CJK IME (physical-position proof); Finder Option menus unaffected; meta+enter cycles three input sources; meta+m overlay appears with no first-frame stall, modes toggle, labels stable within a hold; switching moves the window across Spaces and displays, unminimizes, and degrades to activation-only when AX is denied; a hung app cannot stall the gesture; permission grant flips the engine live within 250 ms; sleep/wake and secure-input windows recover via the health timer; hidutil mapping survives reboot via the LaunchAgent and self-heals after consent revocation; uninstall leaves zero mapping entries and no plist; Activity Monitor shows about zero idle CPU and stable footprint (procwatch); bare-binary launch warns to bundle first; packaged .app keeps its TCC grant across rebuilds.

## Known risks

- The exact hidutil JSON encoding of a non-modifier destination needs on-device verification (hex never does this; C1 flagged it honestly).
- `NSRunningApplication.activate()` binding availability in objc2 on macOS 15 (plain `activate` exists 14+; fall back to `activateWithOptions`).
- Live-list staleness mid-hold (an app launched a second ago may be missing); acceptable, switching still activates windowless apps.
- Sequoia Login-Items consent can silently disable the LaunchAgent; self-heal plus a status badge is the mitigation, and first-run UX includes one consent click.
- The idle-draw budget relies on rune's hidden-overlay pacing; assert it with surface stats during implementation rather than trusting this doc.

## Synthesis note

Base: candidate 3 ("the engine owns everything, rune only draws") — it won the criteria where architecture differs (key-engine correctness, boundaries, minimalism) on both my read and the cross-judge's independent scoring. The judge's verdict: the engine is the highest-risk subsystem and C3 is the only candidate race-free by construction, with route memory as the crispest release-matching pattern.

Grafts folded in by hand:
- From candidate 1: sticky label memory (the goal's "consistent" across activations); pinned-empty boots straight to Live; minimized-window handling (clear `kAXMinimized`); the Finder Option-menu and LED human tests; click-to-cancel backdrop via `OverlayInput::Regions` + `.hit()`.
- From candidate 2: never read `kCGWindowName` (Screen Recording trap); the most complete synthesis spec (explicit down/up plus kept modifier flags); `upstream_eaten` evidence semantics (HID listen-only tap); RESERVED enforced in both validation and assignment (fixes C3's dead-label and m-lockout bugs); honest degradation wording.
- Corrections forced by fact-checking: F18 virtual keycode is `0x4F` (all three candidates were wrong; verified against the SDK's `Events.h`); C2's hidutil destination was F6; label priority re-derived from the actual Colemak-DH rationale (center columns demoted below strong top-row keys); the chord-during-hold corner case now marks `used` so a passed chord kills the pending Esc.
- Deviation from all candidates: overlay cursor navigation (h/j/k/l moving a selection) is dropped from v1; label keys are the entire selection model, which removes the dead-label bug class and shrinks the table. Nav is the first fast-follow.
- Convergence noted: all three independently chose hidutil Caps->F18 + LaunchAgent, decide-Esc-at-release with no timers, a display-only prewarmed overlay, settings.json with hex recovery rules and ArcSwap hot reload, and activate-plus-AX-raise switching. Consensus on those points ships as-is.
- Arena record: three candidates, no dropouts; cross-judge ran on the parent model because this runtime forbids guessing model IDs, so the intended family-diversity check did not run; its factual spot-checks still caught the F18 error class and the label-collision bugs.
- Contested facts verified during synthesis: kVK_F18 = 0x4F and all listed keycodes (SDK `Events.h`); hidden overlays do not draw and pace themselves (rune `overlay` tests and `options.rs`); Colemak-DH rankings (colemakmods.github.io/mod-dh: D and H move to bottom-row index-curl positions, G reverts to its QWERTY placement, H and M exchange, center-column usage halves).
