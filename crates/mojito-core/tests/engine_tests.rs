//! Engine golden sequences: every row of the decision table, driven with
//! injected monotonic timestamps (no real clocks). Public API only.

use arc_swap::ArcSwap;
use mojito_core::bridge::AppSlot;
use mojito_core::{
    Action, AppIndex, AppRef, Config, Engine, Job, ModifierFlags, Mono, PhysicalKey, PinnedApp,
    RawKey, SwitchRequest, SynthKey, UiMsg,
};
use std::sync::Arc;

const H: u16 = 0x04;
const J: u16 = 0x26;
const TAB: u16 = 0x30;
const Q: u16 = 0x0C;
const F: u16 = 0x03;
const SWITCHER: u16 = 0x0E;
const RET: u16 = 0x24;
const ESC: u16 = 0x35;
const F18: u16 = 0x4F;
const TRIGGER: u16 = 0x18; // '=' — not reserved, not in the label pool
const LABEL_FIRST: u16 = 0x03; // f — the head of LABEL_PRIORITY

const ARROW_LEFT: PhysicalKey = PhysicalKey(0x7B);
const ARROW_DOWN: PhysicalKey = PhysicalKey(0x7D);
const BACKSPACE: PhysicalKey = PhysicalKey(0x33);

fn ms(t: u64) -> Mono {
    Mono(t * 1_000_000)
}

fn rig(pinned: Vec<PinnedApp>, slots: Vec<AppSlot>) -> Engine {
    let config = Arc::new(ArcSwap::from_pointee(Config {
        pinned,
        ..Default::default()
    }));
    let index = Arc::new(ArcSwap::from_pointee(AppIndex { slots }));
    Engine::new(config, index)
}

fn standard_rig() -> Engine {
    rig(
        vec![PinnedApp {
            name: "Finder".into(),
            bundle_id: "com.apple.finder".into(),
            key: PhysicalKey(TRIGGER),
        }],
        vec![
            AppSlot {
                pid: 42,
                bundle_id: Some("com.apple.mail".into()),
                name: "Mail".into(),
                windows: 2,
                is_frontmost: false,
            },
            AppSlot {
                pid: 43,
                bundle_id: Some("com.apple.Safari".into()),
                name: "Safari".into(),
                windows: 1,
                is_frontmost: false,
            },
            AppSlot {
                pid: 44,
                bundle_id: Some("com.apple.Notes".into()),
                name: "Notes".into(),
                windows: 3,
                is_frontmost: false,
            },
        ],
    )
}

fn key(code: u16, down: bool, t: u64) -> RawKey {
    RawKey {
        key: PhysicalKey(code),
        down,
        repeat: false,
        flags: ModifierFlags::NONE,
        t: ms(t),
    }
}

fn flagged(code: u16, down: bool, t: u64, flags: ModifierFlags) -> RawKey {
    RawKey {
        key: PhysicalKey(code),
        down,
        repeat: false,
        flags,
        t: ms(t),
    }
}

fn repeated(code: u16, t: u64) -> RawKey {
    RawKey {
        key: PhysicalKey(code),
        down: true,
        repeat: true,
        flags: ModifierFlags::NONE,
        t: ms(t),
    }
}

fn synth(action: &Action) -> Option<SynthKey> {
    match action {
        Action::Synth(s) => Some(*s),
        _ => None,
    }
}

fn ui(action: &Action) -> Option<&UiMsg> {
    match action {
        Action::Ui(msg) => Some(msg),
        _ => None,
    }
}

fn job(action: &Action) -> Option<&Job> {
    match action {
        Action::Job(j) => Some(j),
        _ => None,
    }
}

fn synths(actions: &[Action]) -> Vec<SynthKey> {
    actions.iter().filter_map(synth).collect()
}

#[test]
fn tap_fires_exactly_one_esc_down_and_up() {
    let mut engine = standard_rig();
    assert_eq!(engine.on_event(key(F18, true, 0)), vec![Action::Swallow]);
    let release = engine.on_event(key(F18, false, 50));
    assert_eq!(release[0], Action::Swallow);
    let escs = synths(&release[1..]);
    assert_eq!(escs.len(), 2);
    assert_eq!(
        escs[0],
        SynthKey {
            key: PhysicalKey(ESC),
            down: true,
            flags: ModifierFlags::NONE
        }
    );
    assert_eq!(
        escs[1],
        SynthKey {
            key: PhysicalKey(ESC),
            down: false,
            flags: ModifierFlags::NONE
        }
    );
    // No UI traffic for a bare tap.
    assert!(release.iter().all(|a| ui(a).is_none()));
}

#[test]
fn tap_at_the_exact_threshold_still_fires() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    let release = engine.on_event(key(F18, false, 250)); // tap_max_ms == 250
    assert_eq!(synths(&release).len(), 2);
}

#[test]
fn stale_release_fires_nothing() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    let release = engine.on_event(key(F18, false, 251));
    assert_eq!(release, vec![Action::Swallow]);
}

#[test]
fn f18_autorepeat_does_not_kill_the_tap() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    engine.on_event(repeated(F18, 100)); // the meta key itself, not "a key"
    let release = engine.on_event(key(F18, false, 150));
    assert_eq!(synths(&release).len(), 2);
}

#[test]
fn hold_h_synthesizes_left_down_and_later_its_up() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    let down = engine.on_event(key(H, true, 10));
    assert_eq!(
        down,
        vec![
            Action::Swallow,
            Action::Synth(SynthKey {
                key: ARROW_LEFT,
                down: true,
                flags: ModifierFlags::NONE
            })
        ]
    );
    let up = engine.on_event(key(H, false, 40));
    assert_eq!(
        up,
        vec![
            Action::Swallow,
            Action::Synth(SynthKey {
                key: ARROW_LEFT,
                down: false,
                flags: ModifierFlags::NONE
            })
        ]
    );
    // Releasing meta after a routed key never fires Esc nor a stray Hide.
    assert_eq!(engine.on_event(key(F18, false, 50)), vec![Action::Swallow]);
}

#[test]
fn autorepeat_re_synthesizes_downs_and_one_up_at_release() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    engine.on_event(key(H, true, 10));
    for t in [100, 200, 300] {
        let actions = engine.on_event(repeated(H, t));
        assert_eq!(
            synths(&actions),
            vec![SynthKey {
                key: ARROW_LEFT,
                down: true,
                flags: ModifierFlags::NONE
            }]
        );
    }
    let up = engine.on_event(key(H, false, 310));
    let ups: Vec<SynthKey> = synths(&up).into_iter().filter(|s| !s.down).collect();
    assert_eq!(
        ups,
        vec![SynthKey {
            key: ARROW_LEFT,
            down: false,
            flags: ModifierFlags::NONE
        }]
    );
}

#[test]
fn shift_chord_honored_on_routed_keys() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    let down = engine.on_event(flagged(H, true, 10, ModifierFlags::SHIFT));
    assert_eq!(
        down,
        vec![
            Action::Swallow,
            Action::Synth(SynthKey {
                key: ARROW_LEFT,
                down: true,
                flags: ModifierFlags::SHIFT
            })
        ]
    );
}

#[test]
fn cmd_opt_ctrl_ride_along_on_routed_keys_through_repeat_and_up() {
    let chord = ModifierFlags(
        ModifierFlags::CMD.0 | ModifierFlags::OPT.0 | ModifierFlags::CTRL.0 | ModifierFlags::SHIFT.0,
    );
    let arrow = |down| {
        vec![
            Action::Swallow,
            Action::Synth(SynthKey {
                key: ARROW_LEFT,
                down,
                flags: chord,
            }),
        ]
    };
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    assert_eq!(engine.on_event(flagged(H, true, 10, chord)), arrow(true));
    let repeat = RawKey {
        repeat: true,
        ..flagged(H, true, 20, chord)
    };
    assert_eq!(engine.on_event(repeat), arrow(true));
    assert_eq!(engine.on_event(flagged(H, false, 30, chord)), arrow(false));
}

#[test]
fn meta_enter_toggles_input_once_and_ignores_autorepeat() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    let down = engine.on_event(key(RET, true, 10));
    assert_eq!(job(&down[1]), Some(&Job::ToggleInputSource));
    assert_eq!(engine.on_event(repeated(RET, 100)), vec![Action::Swallow]);
    assert_eq!(engine.on_event(key(RET, false, 120)), vec![Action::Swallow]);
    // The input toggle is decided by the table, not by phase churn after.
    assert_eq!(engine.on_event(key(F18, false, 130)), vec![Action::Swallow]);
}

#[test]
fn meta_m_shows_pinned_rows_and_requests_window_counts() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    let actions = engine.on_event(key(SWITCHER, true, 10));
    assert_eq!(actions[0], Action::Swallow);
    match ui(&actions[1]) {
        Some(UiMsg::ShowPinned { rows }) => {
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].key, PhysicalKey(TRIGGER));
            assert_eq!(rows[0].name, "Finder");
        }
        other => panic!("expected ShowPinned, got {other:?}"),
    }
    assert_eq!(job(&actions[2]), Some(&Job::RefreshWindowCounts));
}

#[test]
fn m_toggles_pinned_live_pinned() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    let pinned = engine.on_event(key(SWITCHER, true, 10));
    assert!(matches!(ui(&pinned[1]), Some(UiMsg::ShowPinned { .. })));
    let live = engine.on_event(key(SWITCHER, true, 20));
    match ui(&live[1]) {
        Some(UiMsg::ShowLive { rows }) => {
            // Frontmost excluded (none here), windowless excluded, recency
            // order preserved, labels assigned from the pool.
            assert_eq!(rows.len(), 3);
            assert_eq!(rows[0].pid, 42);
            assert_eq!(rows[0].label, Some(PhysicalKey(F)));
            assert_eq!(rows[1].label, Some(PhysicalKey(0x02)));
            assert_eq!(rows[2].label, Some(PhysicalKey(0x01)));
        }
        other => panic!("expected ShowLive, got {other:?}"),
    }
    let back = engine.on_event(key(SWITCHER, true, 30));
    assert!(matches!(ui(&back[1]), Some(UiMsg::ShowPinned { .. })));
}

#[test]
fn pinned_trigger_key_is_inert_in_the_live_list() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    engine.on_event(key(SWITCHER, true, 10));
    engine.on_event(key(SWITCHER, true, 20));
    assert_eq!(engine.on_event(key(TRIGGER, true, 30)), vec![Action::Swallow]);
    assert_eq!(engine.on_event(key(TRIGGER, false, 40)), vec![Action::Swallow]);
}

#[test]
fn label_press_commits_and_leftovers_are_inert() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    engine.on_event(key(SWITCHER, true, 10)); // Pinned
    engine.on_event(key(SWITCHER, true, 20)); // Live; Mail (pid 42) owns f
    let commit = engine.on_event(key(F, true, 30));
    assert_eq!(commit[0], Action::Swallow);
    assert_eq!(ui(&commit[1]), Some(&UiMsg::Hide));
    assert_eq!(
        job(&commit[2]),
        Some(&Job::Switch(SwitchRequest {
            app: AppRef::Pid(42)
        }))
    );
    // Leftovers after the commit: swallowed, no synth, no switch, no typing.
    assert_eq!(engine.on_event(repeated(F, 40)), vec![Action::Swallow]);
    assert_eq!(engine.on_event(key(H, true, 50)), vec![Action::Swallow]);
    assert_eq!(engine.on_event(key(F, false, 60)), vec![Action::Swallow]);
    assert_eq!(engine.on_event(key(H, false, 70)), vec![Action::Swallow]);
    // Release after commit: overlay already hidden, no Esc, back to Idle.
    assert_eq!(engine.on_event(key(F18, false, 80)), vec![Action::Swallow]);
    assert_eq!(engine.on_event(key(Q, true, 90)), vec![Action::Pass]);
}

#[test]
fn pin_trigger_commits_a_bundle_switch() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    engine.on_event(key(SWITCHER, true, 10));
    let commit = engine.on_event(key(TRIGGER, true, 20));
    assert_eq!(ui(&commit[1]), Some(&UiMsg::Hide));
    assert_eq!(
        job(&commit[2]),
        Some(&Job::Switch(SwitchRequest {
            app: AppRef::Bundle("com.apple.finder".into())
        }))
    );
    assert_eq!(
        engine.on_event(repeated(TRIGGER, 30)),
        vec![Action::Swallow]
    );
    assert_eq!(
        engine.on_event(key(TRIGGER, false, 40)),
        vec![Action::Swallow]
    );
    assert_eq!(engine.on_event(key(F18, false, 50)), vec![Action::Swallow]);
}

#[test]
fn release_leaves_the_overlay_open_and_bare_keys_still_select() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    engine.on_event(key(SWITCHER, true, 10));
    // Releasing meta keeps the switcher: no Hide, no Esc.
    assert_eq!(engine.on_event(key(F18, false, 100)), vec![Action::Swallow]);
    // A bare e toggles to the live list, then a bare label key selects.
    engine.on_event(key(SWITCHER, true, 105));
    let commit = engine.on_event(key(LABEL_FIRST, true, 110));
    assert_eq!(commit[0], Action::Swallow);
    assert_eq!(ui(&commit[1]), Some(&UiMsg::Hide));
    assert!(matches!(&commit[2], Action::Job(Job::Switch(_))));
    // The machine is transparent again for plain typing.
    let plain = engine.on_event(key(LABEL_FIRST, false, 115));
    assert!(plain.iter().all(|a| ui(a).is_none()));
}

#[test]
fn sticky_overlay_esc_closes_and_unbound_keys_are_swallowed() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    engine.on_event(key(SWITCHER, true, 10));
    engine.on_event(key(F18, false, 20));
    // An unbound bare key does nothing but is still ours.
    assert_eq!(engine.on_event(key(0x2C, true, 30)), vec![Action::Swallow]);
    // Esc without meta closes the switcher with a fade.
    let cancel = engine.on_event(key(ESC, true, 40));
    assert_eq!(cancel, vec![Action::Swallow, Action::Ui(UiMsg::HideFade)]);
    // Transparent again: a fresh key passes (the earlier '/' up stays
    // swallowed because its down was ours).
    assert_eq!(engine.on_event(key(Q, true, 50)), vec![Action::Pass]);
}

#[test]
fn sticky_overlay_toggles_modes_without_meta() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    engine.on_event(key(SWITCHER, true, 10));
    engine.on_event(key(F18, false, 20));
    let toggle = engine.on_event(key(SWITCHER, true, 30));
    assert!(matches!(ui(&toggle[1]), Some(UiMsg::ShowLive { .. })));
    let back = engine.on_event(key(SWITCHER, true, 40));
    assert!(matches!(ui(&back[1]), Some(UiMsg::ShowPinned { .. })));
}

#[test]
fn commit_without_meta_returns_to_idle() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    engine.on_event(key(SWITCHER, true, 10));
    engine.on_event(key(F18, false, 20));
    let commit = engine.on_event(key(TRIGGER, true, 30));
    assert_eq!(ui(&commit[1]), Some(&UiMsg::Hide));
    // Plain typing passes immediately after a sticky commit.
    let after = engine.on_event(key(0x2C, true, 40));
    assert_eq!(after, vec![Action::Pass]);
}

#[test]
fn commit_with_meta_held_stays_inert_until_release() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    engine.on_event(key(SWITCHER, true, 10));
    let commit = engine.on_event(key(TRIGGER, true, 20));
    assert_eq!(ui(&commit[1]), Some(&UiMsg::Hide));
    // The rest of the hold is inert: no re-trigger, no typing.
    assert_eq!(engine.on_event(key(TRIGGER, true, 30)), vec![Action::Swallow]);
    assert_eq!(engine.on_event(key(F18, false, 40)), vec![Action::Swallow]);
    // Idle again.
    assert_eq!(engine.on_event(key(0x2C, true, 50)), vec![Action::Pass]);
}

#[test]
fn cancel_overlay_control_only_reaches_an_open_overlay() {
    let mut engine = standard_rig();
    // Nothing open: the control path is a no-op.
    assert_eq!(engine.cancel_overlay(), vec![]);
    engine.on_event(key(F18, true, 0));
    engine.on_event(key(SWITCHER, true, 10));
    // Meta still held: cancel lands back on the chord layer.
    let cancel = engine.cancel_overlay();
    assert_eq!(cancel, vec![Action::Ui(UiMsg::HideFade)]);
    let arrow = engine.on_event(key(H, true, 20));
    assert!(matches!(arrow.as_slice(), [Action::Swallow, Action::Synth(_)]));
}

#[test]
fn cmd_chord_passes_during_sticky_overlay_too() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    engine.on_event(key(SWITCHER, true, 10));
    engine.on_event(key(F18, false, 20));
    let down = engine.on_event(flagged(TAB, true, 30, ModifierFlags::CMD));
    assert_eq!(down, vec![Action::Pass]);
    assert_eq!(
        engine.on_event(flagged(TAB, false, 40, ModifierFlags::CMD)),
        vec![Action::Pass]
    );
}

#[test]
fn esc_cancels_with_a_fade_and_the_hold_survives() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    engine.on_event(key(SWITCHER, true, 10));
    let cancel = engine.on_event(key(ESC, true, 20));
    assert_eq!(cancel, vec![Action::Swallow, Action::Ui(UiMsg::HideFade)]);
    // Cancel returns to the hidden-meta layer: esc itself was swallowed,
    // release hides nothing (already hidden), and e reopens the overlay.
    assert_eq!(engine.on_event(key(ESC, false, 25)), vec![Action::Swallow]);
    assert_eq!(engine.on_event(key(F18, false, 30)), vec![Action::Swallow]);
    let reopen = {
        engine.on_event(key(F18, true, 40));
        engine.on_event(key(SWITCHER, true, 50))
    };
    assert!(matches!(ui(&reopen[1]), Some(UiMsg::ShowPinned { .. })));
}

#[test]
fn cmd_chord_passes_during_hold_and_marks_used() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    let down = engine.on_event(flagged(TAB, true, 10, ModifierFlags::CMD));
    assert_eq!(down, vec![Action::Pass]);
    let up = engine.on_event(flagged(TAB, false, 20, ModifierFlags::CMD));
    assert_eq!(up, vec![Action::Pass]);
    assert_eq!(engine.on_event(key(F18, false, 30)), vec![Action::Swallow]); // no Esc
}

#[test]
fn opt_chord_passes_during_hold_too() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    assert_eq!(
        engine.on_event(flagged(Q, true, 10, ModifierFlags::OPT)),
        vec![Action::Pass]
    );
    assert_eq!(engine.on_event(key(F18, false, 20)), vec![Action::Swallow]);
}

#[test]
fn unbound_bare_key_is_swallowed_and_kills_the_tap() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    assert_eq!(engine.on_event(key(Q, true, 10)), vec![Action::Swallow]);
    assert_eq!(engine.on_event(key(Q, false, 20)), vec![Action::Swallow]);
    assert_eq!(engine.on_event(key(F18, false, 30)), vec![Action::Swallow]); // no Esc
}

#[test]
fn j_maps_to_arrow_down_and_i_to_backspace() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    assert_eq!(
        synths(&engine.on_event(key(J, true, 10))),
        vec![SynthKey {
            key: ARROW_DOWN,
            down: true,
            flags: ModifierFlags::NONE
        }]
    );
    assert_eq!(
        synths(&engine.on_event(key(0x22, true, 20))),
        vec![SynthKey {
            key: BACKSPACE,
            down: true,
            flags: ModifierFlags::NONE
        }]
    );
}

#[test]
fn up_obeys_down_even_after_release() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    engine.on_event(key(H, true, 10)); // synth Left down
    engine.on_event(key(F18, false, 20)); // release; hold over, route persists
    let up = engine.on_event(key(H, false, 30));
    assert_eq!(
        up,
        vec![
            Action::Swallow,
            Action::Synth(SynthKey {
                key: ARROW_LEFT,
                down: false,
                flags: ModifierFlags::NONE
            })
        ]
    );
}

#[test]
fn passed_chord_up_obays_its_pass_even_after_release() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    engine.on_event(flagged(TAB, true, 10, ModifierFlags::CMD));
    engine.on_event(key(F18, false, 20));
    assert_eq!(
        engine.on_event(flagged(TAB, false, 30, ModifierFlags::CMD)),
        vec![Action::Pass]
    );
}

#[test]
fn a_key_down_before_the_hold_still_passes_its_up_during_the_hold() {
    let mut engine = standard_rig();
    engine.on_event(key(Q, true, 0)); // Idle: passes, route remembered
    engine.on_event(key(F18, true, 10)); // hold begins under the held key
    let up = engine.on_event(key(Q, false, 20));
    assert_eq!(up, vec![Action::Pass]);
    // And the chord marked the hold used: no Esc on release.
    assert_eq!(engine.on_event(key(F18, false, 30)), vec![Action::Swallow]);
}

#[test]
fn empty_pin_list_boots_straight_to_live() {
    let mut engine = rig(
        vec![],
        vec![AppSlot {
            pid: 7,
            bundle_id: Some("com.a".into()),
            name: "A".into(),
            windows: 1,
            is_frontmost: false,
        }],
    );
    engine.on_event(key(F18, true, 0));
    let open = engine.on_event(key(SWITCHER, true, 10));
    assert!(matches!(ui(&open[1]), Some(UiMsg::ShowLive { .. })));
}

#[test]
fn frontmost_and_windowless_apps_never_appear_in_live() {
    let mut engine = rig(
        vec![],
        vec![
            AppSlot {
                pid: 1,
                bundle_id: Some("com.front".into()),
                name: "Front".into(),
                windows: 5,
                is_frontmost: true,
            },
            AppSlot {
                pid: 2,
                bundle_id: Some("com.hidden".into()),
                name: "Hidden".into(),
                windows: 0,
                is_frontmost: false,
            },
            AppSlot {
                pid: 3,
                bundle_id: Some("com.real".into()),
                name: "Real".into(),
                windows: 1,
                is_frontmost: false,
            },
        ],
    );
    engine.on_event(key(F18, true, 0));
    let open = engine.on_event(key(SWITCHER, true, 10));
    match ui(&open[1]) {
        Some(UiMsg::ShowLive { rows }) => {
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].pid, 3);
        }
        other => panic!("expected ShowLive, got {other:?}"),
    }
}

#[test]
fn reset_from_any_state_returns_to_idle_and_hides() {
    // From Idle.
    let mut engine = standard_rig();
    assert_eq!(engine.reset(), vec![Action::Ui(UiMsg::Hide)]);
    assert_eq!(engine.on_event(key(Q, true, 0)), vec![Action::Pass]);

    // From Armed.
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    assert_eq!(engine.reset(), vec![Action::Ui(UiMsg::Hide)]);
    assert_eq!(engine.on_event(key(F18, false, 10)), vec![Action::Swallow]); // no Esc

    // From Pinned overlay.
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    engine.on_event(key(SWITCHER, true, 10));
    assert_eq!(engine.reset(), vec![Action::Ui(UiMsg::Hide)]);
    assert_eq!(engine.on_event(key(F18, false, 20)), vec![Action::Swallow]); // no Esc, no Hide
    assert_eq!(engine.on_event(key(Q, true, 30)), vec![Action::Pass]);
}

#[test]
fn settings_save_rebuilds_tables_on_the_next_gesture() {
    let config = Arc::new(ArcSwap::from_pointee(Config::default()));
    let index = Arc::new(ArcSwap::from_pointee(AppIndex::default()));
    let mut engine = Engine::new(config.clone(), index);
    engine.on_event(key(F18, true, 0));
    // Simulate a settings save: the store bumps the version, the engine
    // rebuilds its tables lazily on the next event.
    config.store(Arc::new(Config {
        pinned: vec![PinnedApp {
            name: "Terminal".into(),
            bundle_id: "com.apple.Terminal".into(),
            key: PhysicalKey(0x07),
        }],
        hint_layout: None,
        version: 1,
        ..Default::default()
    }));
    let open = engine.on_event(key(SWITCHER, true, 10));
    match ui(&open[1]) {
        Some(UiMsg::ShowPinned { rows }) => {
            assert_eq!(rows[0].name, "Terminal");
            assert_eq!(rows[0].key, PhysicalKey(0x07));
        }
        other => panic!("expected ShowPinned, got {other:?}"),
    }
}

// The backend moves the engine to the tap thread; keep that true.
fn _engine_must_be_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Engine>();
    assert_send::<RawKey>();
    assert_send::<Action>();
}

// Three filter taps (HID head, session, annotated session) deliver one
// hardware event up to three times; every delivery after the first must
// repeat the first decision without touching the machine.
#[test]
fn duplicate_events_repeat_the_decision_without_side_effects() {
    let mut engine = standard_rig();

    // Idle pass-through: duplicated, and the route up still passes.
    let a = key(0x00, true, 1_000);
    assert_eq!(engine.on_event(a), vec![Action::Pass]);
    assert_eq!(engine.on_event(a), vec![Action::Pass]);
    assert_eq!(engine.on_event(key(0x00, false, 1_100)), vec![Action::Pass]);

    // A hold that opens the overlay: the duplicated e-down must not open it
    // twice (a second decision here would toggle straight to the live list).
    assert_eq!(engine.on_event(key(F18, true, 2_000)), vec![Action::Swallow]);
    let open = engine.on_event(key(SWITCHER, true, 2_050));
    assert!(matches!(ui(&open[1]), Some(UiMsg::ShowPinned { .. })));
    assert_eq!(engine.on_event(key(SWITCHER, true, 2_050)), vec![Action::Swallow]);

    // The sticky overlay survives meta release; the duplicated Esc-down must
    // not fade twice.
    assert_eq!(engine.on_event(key(F18, false, 2_100)), vec![Action::Swallow]);
    let cancel = engine.on_event(key(ESC, true, 3_000));
    assert_eq!(cancel.len(), 2);
    assert_eq!(cancel[0], Action::Swallow);
    assert!(matches!(ui(&cancel[1]), Some(UiMsg::HideFade)));
    assert_eq!(engine.on_event(key(ESC, true, 3_000)), vec![Action::Swallow]);
    assert_eq!(engine.on_event(key(ESC, false, 3_050)), vec![Action::Swallow]);

    // Idle again: a fresh Esc passes, and a fresh meta tap still synthesizes
    // Esc — no stale overlay state.
    assert_eq!(engine.on_event(key(ESC, true, 4_000)), vec![Action::Pass]);
    assert_eq!(engine.on_event(key(F18, true, 4_100)), vec![Action::Swallow]);
    let tap_esc = engine.on_event(key(F18, false, 4_200));
    assert_eq!(
        synth(&tap_esc[1]),
        Some(SynthKey {
            key: PhysicalKey(ESC),
            down: true,
            flags: ModifierFlags::NONE,
        })
    );
}

// A bare second caps press over the open overlay dismisses it.
#[test]
fn tapping_caps_again_dismisses_the_open_overlay() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    engine.on_event(key(SWITCHER, true, 50)); // ShowPinned
    engine.on_event(key(F18, false, 100)); // sticky overlay

    // The second caps press and release: fade, then Idle.
    assert_eq!(engine.on_event(key(F18, true, 5_000)), vec![Action::Swallow]);
    let up = engine.on_event(key(F18, false, 5_100));
    assert_eq!(up.len(), 2);
    assert_eq!(up[0], Action::Swallow);
    assert!(matches!(ui(&up[1]), Some(UiMsg::HideFade)));

    // Idle again: a caps tap synthesizes Esc as always.
    assert_eq!(engine.on_event(key(F18, true, 6_000)), vec![Action::Swallow]);
    let tap = engine.on_event(key(F18, false, 6_100));
    assert_eq!(
        synth(&tap[1]),
        Some(SynthKey {
            key: PhysicalKey(ESC),
            down: true,
            flags: ModifierFlags::NONE,
        })
    );
}

// Using a key during the second hold keeps the overlay open after release.
#[test]
fn using_a_key_during_the_second_hold_keeps_the_overlay() {
    let mut engine = standard_rig();
    engine.on_event(key(F18, true, 0));
    engine.on_event(key(SWITCHER, true, 50));
    engine.on_event(key(F18, false, 100));

    assert_eq!(engine.on_event(key(F18, true, 5_000)), vec![Action::Swallow]);
    let toggle = engine.on_event(key(SWITCHER, true, 5_050)); // e -> live list
    assert!(matches!(ui(&toggle[1]), Some(UiMsg::ShowLive { .. })));
    assert_eq!(engine.on_event(key(F18, false, 5_200)), vec![Action::Swallow]);
    // Still in the overlay: Esc cancels with the fade.
    let esc = engine.on_event(key(ESC, true, 5_400));
    assert!(matches!(ui(&esc[1]), Some(UiMsg::HideFade)));
}
