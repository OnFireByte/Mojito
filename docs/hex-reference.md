# hex systems reference

Reference for agents working on macOS plumbing in this repo. Distilled on 2026-11-01 from a full read of `anomalyco/hex` branch `dev` at rev `cc286a09a2dac6d6f0711074dbcef80a5fd28097`. Paths point into that checkout. Hex is a shipped macOS voice dictation app in Rust on the rune UI framework, so its solutions are proven in production. Verify anything you copy against the pinned rev.

## What to copy for Mojito, and what hex lacks

Hex has proven patterns for event taps, key synthesis, accessibility permissions, overlay windows, settings persistence, packaging, and input-source observation. Hex has nothing for window management (no `CGWindowList`, no `AXRaise`, no activation of other apps) and never selects an input source (no `TISSelectInputSource`). Mojito needs both, so those two systems are original work built from the primitives hex does show: bounded AX calls, `NSRunningApplication`, and the TIS list filter.

## Packaging a runnable debug .app

- Hex builds one binary crate with per-OS modules behind `#[cfg]` (`src/main.rs:5-62`). Its `build.rs` compiles Swift bridges and an ObjC permission-guide file; Mojito can skip the Swift parts.
- `.app` assembly in `scripts/build-app.sh`: `cargo build --release`, copy the binary to `Mojito.app/Contents/MacOS/`, copy and patch `app/Info.plist` with PlistBuddy, compile assets, then `codesign --force --sign - Mojito.app` (ad-hoc, `build-app.sh:15-19`).
- `app/Info.plist` keys that matter: `CFBundleIdentifier`, `CFBundleExecutable`, `LSUIElement=true` (no Dock icon), `LSMinimumSystemVersion 15.0`, `NSAccessibilityUsageDescription` (`app/Info.plist:36-43`).
- Accessibility is a TCC permission, not an entitlement. Hex's release entitlements cover only audio, Apple events, and calendars (`.github/release/hex.entitlements`).
- Bundle id is what TCC attaches the grant to. Bundle the app before first run, or every rebuilt binary path re-prompts.
- Minimum recipe: build the binary, create `Mojito.app/Contents/{MacOS,Resources}`, write Info.plist, ad-hoc sign, launch with `open Mojito.app`. No SMAppService or notarization until a real team signs and installs to `/Applications` (`AGENTS.md:1402-1409`).

## The event tap engine (`src/suppression.rs`, `src/keyboard.rs`)

Hex declares its CoreGraphics bindings with `unsafe extern "C"` blocks in-file (`suppression.rs:72-136`) instead of pulling a key-crate dependency. The load-bearing design:

- Three taps on one dedicated thread (`suppression.rs:627-653`):
  - Key tap: `CGEventTapCreate(kCGAnnotatedSessionEventTap, kCGHeadInsertEventTap, kCGEventTapOptionDefault, KEY_MASK, cb)`, actively filtering.
  - Observation tap: same location but `kCGEventTapOptionListenOnly`, watching `flagsChanged` only. A filtering tap on flagsChanged breaks Finder's Option-driven alternate menus even when events pass through unchanged (comment at 635-641).
  - Physical tap: `kCGHIDEventTap`, listen-only, key-down mask, used as chord evidence when another app consumed the key above the session tap.
- Tap thread setup: `CFMachPortCreateRunLoopSource`, `CFRunLoopAddSource(kCFRunLoopCommonModes)`, `CGEventTapEnable(true)`, then `CFRunLoopRun()`. Readiness signaled over a `sync_channel` with a 2 s timeout (`AttachedTap::create` at 396-433, `InputMonitor::start` at 312-344).
- Callbacks never block. Events go over `mpsc`; if the receiver is gone the callback stops the run loop (`1316-1332`).
- Key theft control: the callback returns events unchanged unless the hotkey matched and was delivered to the listener; then it returns `ptr::null_mut()` to swallow (`1096-1100`). A `HashMap<u16, bool>` remembers each key-down's suppress decision so autorepeats and releases inherit it (`1334-1338`). Fresh down edges detected via `pressed_keys.insert` returning true, not via the autorepeat field.
- Tap-vs-hold reference: `DictationHotkey` state machine with `State::{Idle, FirstTapPressed, AwaitingSecondTap, SecondTapPressed, Recording, Locked, Dirty}` (`suppression.rs:1567-2412`). Hold threshold 300 ms (`src/dictation.rs:38`). Stale-state recovery requires 100 ms of keyboard neutrality sampled via `CGEventSourceFlagsState(kCGEventSourceStateCombinedSessionState)` (`1738-1813`).
- The chord-breaker trick, which is the core of hold-a-key-as-modifier: once the engine owns a modifier press, it posts one event on unassigned keycode `0x44` carrying a synthetic marker, at the HID tap, and the session tap swallows it before the focused app sees it (`src/keyboard.rs:24-31, 298-318`; `suppression.rs:1941-2005`). Without this, every app with a modifier hotkey fires and the focused app beeps.
- Synthesis: `CGEventCreateKeyboardEvent(null, keycode, down)` plus `CGEventSetFlags` plus `CGEventSetIntegerValueField(kCGKeyboardEventIntegerFieldEventSourceUserData = 42, MARKER)` plus `CGEventPost(kCGHIDEventTap = 0, event)` (`src/keyboard.rs:66-73, 350-375`). Marker value used by hex: `0x0056_4f49_4345`. Mark every synthetic event and check for the marker in your own tap so you never intercept your own output.
- Keycode resolution for printable keys: `TISCopyCurrentKeyboardLayoutInputSource` + `TISGetInputSourceProperty(kTISPropertyUnicodeKeyLayoutData)` + `UCKeyTranslate` over keycodes 0..128, snapshotted once on the main thread at startup (`154-184`). TIS is not thread-safe; overlapping lookups crashed hex in release 2.1.12 (`docs/releases/2.1.12.md`). Guard with a mutex and never re-enter TIS on a cache miss from a worker thread.
- Resilience: a 5 s run-loop health timer re-enables taps macOS silently disabled and recreates invalidated ones, then emits a reset event so the state machine clears (`571-575, 670-734, 814-829`). Opt the tap thread out of App Nap with `NSProcessInfo beginActivityWithOptions(UserInitiatedAllowingIdleSystemSleep)` (`792-812`).
- Timestamps: session-tap events carry nanoseconds; HID-tap events carried raw Mach ticks on Apple Silicon but ns for some events. Sniff which reading is recent (under 5 s old) and never re-apply a conversion (`907-921`, war story in `docs/releases/2.1.8.md`).

## Accessibility permission

- Detect with both `CGPreflightPostEventAccess()` (launch-time answer) and `AXIsProcessTrusted()` (live answer) (`src/desktop/onboarding.rs:1015-1024, 1171-1211`).
- Prompt with `AXIsProcessTrustedWithOptions(@{kAXTrustedCheckOptionPrompt: @YES})`, wrapped in a small ObjC file and a floating guide window that docks beside System Settings (`native/permission_guide.m:75-78`, `desktop/permission_guide.rs:60-64`).
- Accessibility also grants listen-event access. Hex never requests Input Monitoring (`AGENTS.md:556-559`).
- Poll permission state on a 250 ms runtime tick and start the engine the moment it flips (`src/macos_rune.rs:352-398, 749-755`). The settings pane re-reads state once per second.
- If the user returns from System Settings still untrusted after about 15 s, offer a `tccutil` reset and re-prompt (`StalePermissionTracker`, `onboarding.rs:706-756`; `macos_host.rs:574-607`). macOS 27 renamed the Accessibility settings pane, so resolve the pane name at runtime (`permission_guide.rs:40-44`).

## Overlay windows with rune

- Prewarmed HUD pattern (`src/rune_hud.rs:299-353`): `app.overlay(OverlaySpec { .. }.options(OverlayOptions { prewarm: true, .. }), state)` with `.srgb_blend(true)`. `prewarm` means the first press never waits on the panel.
- Interactive panels that never steal focus: same call with `interactive: true` (`src/rune_ui/meeting_capsules.rs:370-397`). Buttons take clicks while the frontmost app keeps focus.
- Window level: `NSStatusWindowLevel = 25`, plus 1 when over the menu bar, via `overlay.set_macos_level(25 + n)` (`src/rune_hud/macos.rs:17-18, 159-162`).
- Multi-screen: pointer screen from `NSEvent::mouseLocation()` plus `NSScreen::screens` with `visibleFrame` and `safeAreaInsets` (`rune_hud/macos.rs:23-55`). Placement math is pure functions in `rune_hud/hud_placement.rs`. Feed results to rune as `overlay.set_display(DisplayTarget::Primary)` and `set_placement(Some(OverlayPlacement::Rect { x, y, width, height }))` in global points, y-down from the primary display top-left (`166-175`).
- Reveal with content ready: set state and placement and call `show` inside one `overlay.update(...)` using `tx.instant()` (`src/rune_ui/live_text.rs:92-121`). Hidden prewarmed overlays keep drawing so macOS refreshes them before reveal.
- A 16 ms `NSTimer` pump on the main run loop in `NSRunLoopCommonModes` drives native work while animating and keeps firing while menus are open (`src/macos_rune.rs:62-77, 903-937`).
- All Spaces: rune overlays join all Spaces already. Do not try `MoveToActiveSpace` on them beside `CanJoinAllSpaces`; AppKit refuses the combination (`macos_rune.rs:967-984`).
- Screen-share hygiene: a non-shareable window blanks Mission Control thumbnails. Prewarmed overlays at alpha 0 stay shareable; set `exclude_from_capture` deliberately (`AGENTS.md:953-955`).

## App wiring and threads

- The main thread runs rune's AppKit event loop: `rune_ui::app(..).accessory().windowless().keep_running()..run()` (`src/macos_rune.rs:280-560`). No separate NSApplication plumbing needed.
- Native ObjC through `objc2` with `define_class!` for delegate targets, `msg_send!` for calls (`macos_rune.rs:1254-1313`). NSStatusItem built the same way (`macos_rune/status_item.rs:46-120`), actions posted over a `SyncSender` into the pump.
- Background threads (`hex-listener`, `hex-runtime`, workers) talk over `mpsc` and bounded `sync_channel`s. Shared state is `Arc<Published>` swapped under a mutex with registered wake closures (`136-140, 588-618`); the UI subscribes a wake that triggers a rune redraw.
- Deferred setup: nothing the first frame needs may wait on status item or HUD setup (`AGENTS.md:594-606`). Start those in `on_ready` plus a 1 s fallback timer (`macos_rune.rs:529-551, 852-860`).
- Quit discipline: observe `NSApplicationWillTerminateNotification` before the loop starts; on quit, flip a shutdown flag, join workers bounded by 5 s, then `_exit(0)` so C++ static destructors never run under live threads (`macos_rune.rs:498-528`; `src/desktop/process_exit.rs`).

## Settings

- One `settings.json` under `~/Library/Application Support/<app>/`, serde JSON, pretty (`src/app_settings.rs`, `src/app_paths.rs`).
- Recovery rules (`src/app_settings/persistence.rs:38-179`): unknown keys ignored; an invalid field defaults in memory with a per-key warning; an unreadable file loads defaults and leaves the original untouched; before a save over invalid settings, preserve `settings.json.unreadable-<sha256>`, and a failed backup blocks the save.
- Writes go through a durable writer: fsync file and directory, single mutex (`src/durable.rs`).
- Hot reload: `save()` writes the file then pushes values into process-wide atomics and locks (`app_settings.rs:2215-2323`). The runtime re-reads hotkeys per event and rebinds without restarting (`src/recognition.rs:295-368`). Mojito should follow this shape so trigger-key edits apply live.
- Shortcut capture in the settings window: hex suspends its global tap (`set_hotkey_capture_active(true)`) and installs an `NSEvent` local monitor (`addLocalMonitorForEventsMatchingMask_handler`) to capture chords, then converts them to a binding struct and checks conflicts (`src/rune_ui/macos_host.rs:160-337`). Note rune-kit also ships a `ShortcutRecorder` component; compare both before choosing.
- Launch at login: `SMAppService::mainAppService().register()/unregister()/status()` on a dedicated worker thread because it can block; `RequiresApproval` means open System Settings Login Items; macOS state is the source of truth, never persisted (`src/desktop/login_item.rs:1-60, 79-137`).
- Dock policy: `NSApplication::setActivationPolicy(Accessory|Regular)` per a setting; an open window forces the Dock icon on (`app_settings.rs:2913-2935`).

## Input sources (meta+enter cycling)

- Observation module: `src/keyboard_input_sources/macos.rs:87-134`, main-thread only.
- Current source: `TISCopyCurrentKeyboardInputSource()` then `kTISPropertyInputSourceID`.
- Enabled sources: `TISCreateInputSourceList(null, false)` (second arg false means enabled, not installed), filtered to `kTISPropertyInputSourceCategory == kTISCategoryKeyboardInputSource` and `kTISPropertyInputSourceIsSelectCapable == true`, collecting `InputSourceID`, `LocalizedName`, and languages.
- Change watch: distributed notification `kTISNotifySelectedKeyboardInputSourceChanged` on `CFNotificationCenterGetDistributedCenter` sets a dirty flag; the pump drains it (`166-206`, `macos_rune.rs:903-916`).
- Gap Mojito fills: call `TISSelectInputSource(source)` on the main thread to cycle. Build the ordered list with the same filter, find the current index, select the next.

## App enumeration and switching (original work for Mojito)

- Installed-app scanning to crib: `src/application_catalog/macos.rs` walks `/Applications`, `/System/Applications`, `/System/Library/CoreServices/Applications`, `~/Applications` for `.app` bundles; names from `NSFileManager::displayNameAtPath` minus `.app`; bundle ids from `NSBundle::bundleWithPath` (`26-88`). Cheap, no permissions.
- Frontmost app: `NSWorkspace::sharedWorkspace().frontmostApplication()` gives `localizedName`, `bundleIdentifier`, `processIdentifier` (`src/dictation/context.rs:186-239`).
- Running app lookup: `NSRunningApplication::runningApplicationWithProcessIdentifier(pid)` and `runningApplicationsWithBundleIdentifier` (`src/microphone_activity.rs:353-364`).
- AX pattern with hard timeouts: `AXUIElementCreateApplication(pid)`, `AXUIElementCopyAttributeValue(kAXFocusedWindowAttribute / kAXTitleAttribute)`, and `AXUIElementSetMessagingTimeout(element, 0.25)` on every element so a hung app cannot stall the caller (`src/accessibility.rs:9, 64-69, 158-179`).

## Design system notes

- Hex's spec is `docs/design-system.md`: stack is `rune-kit` components plus `rune-theme-opencode` tokens, no color literals outside the theme, one accent fill per screen, hover is instant and only on single-click targets, settings rows never hover-fill, layout is a deep-background sidebar with a raised rounded content panel, window 980x760 min 860x560.
- Visual snapshots: headless `--snap` scene fixtures with light and dark golden images (`rune_ui.rs`, `tests/golden/`).
- Copy rules: sentence case, "…" ellipsis, one product name.

## Performance discipline

- `procwatch` (from the rune repo at the same rev) watches process footprint and CPU every 10 s, 2 s in debug, and warns on growth (`macos_rune.rs:269-278`). Debug builds swap in a counting allocator (`main.rs:92-95`). Worth copying for the resource budget.
- Cadences in hex: runtime tick 250 ms, pump 16 ms only while needed, settings pane refresh 1 s, UI window tick 250 ms.
- Repo rules worth adopting (`AGENTS.md`): files under 800 non-test lines, `scripts/check` runs fmt plus clippy `-D warnings` plus nextest, `strip = true` in release, crash-safe writes everywhere.

## War stories (already-solved failure modes)

- macOS silently disables event taps after sleep, secure input, or accessibility changes. Health timer plus re-enable plus recreate, and reset the state machine when it happens.
- Never filter `flagsChanged` with an active tap; it breaks Finder alternate menus. Listen-only there.
- Carbon global shortcuts in other apps consume keys before session taps. Keep the HID listen-only tap as evidence and discard pending gestures when unknown keys appear.
- TIS calls from worker threads crash. Main-thread snapshot plus mutex.
- AppKit terminate calls `exit` right after will-terminate. Install the observer before the loop runs.
- `SMAppService` needs a signed app in `/Applications`; when replacing a test bundle outside Finder, run `lsregister -f` first.
- Tiling window manager users: open the settings window fixed-size so AeroSpace or yabai floats it instead of tiling it.
- Settings errors must never block launch. Per-key defaults in memory, backups before destructive saves.
