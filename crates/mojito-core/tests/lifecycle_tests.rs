use std::cell::{Cell, RefCell};
use std::time::Duration;

use mojito_core::lifecycle::{GRANT_POLL_INTERVAL, Host, Lifecycle, Next, Remap, set_capture};
use mojito_core::{Config, SwitcherTrigger};

#[derive(Default)]
struct FakeRemap {
    active: Cell<bool>,
    agent: Cell<bool>,
    install_fails: bool,
    calls: RefCell<Vec<&'static str>>,
}

impl Remap for FakeRemap {
    fn is_active(&self) -> bool {
        self.active.get()
    }

    fn install(&self) -> Result<(), String> {
        self.calls.borrow_mut().push("install");
        if self.install_fails {
            return Err("hidutil refused".into());
        }
        self.active.set(true);
        Ok(())
    }

    fn uninstall(&self) -> Result<(), String> {
        self.calls.borrow_mut().push("uninstall");
        self.active.set(false);
        self.agent.set(false);
        Ok(())
    }

    fn ensure_agent(&self) -> Result<(), String> {
        self.calls.borrow_mut().push("ensure_agent");
        self.agent.set(true);
        Ok(())
    }
}

#[derive(Default)]
struct FakeHost {
    remap: FakeRemap,
    granted: bool,
    prompts: u32,
    tap_starts: u32,
    taps_fail: bool,
    native_cmd_tab: Option<bool>,
    attention: Option<bool>,
    reports: Vec<String>,
}

impl Host for FakeHost {
    fn remap(&self) -> &dyn Remap {
        &self.remap
    }

    fn grant_ready(&self) -> bool {
        self.granted
    }

    fn prompt_for_grant(&mut self) {
        self.prompts += 1;
    }

    fn start_taps(&mut self) -> Result<(), String> {
        self.tap_starts += 1;
        if self.taps_fail || !self.granted {
            return Err("no Accessibility grant".into());
        }
        Ok(())
    }

    fn set_native_cmd_tab(&mut self, enabled: bool) {
        self.native_cmd_tab = Some(enabled);
    }

    fn set_needs_attention(&mut self, flag: bool) {
        self.attention = Some(flag);
    }

    fn report(&mut self, context: &str, error: &str) {
        self.reports.push(format!("{context}: {error}"));
    }
}

fn config(trigger: SwitcherTrigger, capture: bool) -> Config {
    Config {
        switcher_trigger: trigger,
        capture_enabled: capture,
        ..Config::default()
    }
}

fn granted_host() -> FakeHost {
    FakeHost {
        granted: true,
        ..FakeHost::default()
    }
}

#[test]
fn granted_start_installs_the_remap_with_its_agent_and_takes_cmd_tab() {
    let mut host = granted_host();
    let next = Lifecycle::default().start(&mut host, &config(SwitcherTrigger::Both, true));
    assert_eq!(next, Next::Settled);
    assert!(host.remap.active.get());
    assert!(host.remap.agent.get());
    assert_eq!(host.prompts, 0);
    assert_eq!(host.native_cmd_tab, Some(false));
    assert_eq!(host.attention, None);
}

#[test]
fn caps_e_only_leaves_the_native_switcher_on() {
    let mut host = granted_host();
    let _ = Lifecycle::default().start(&mut host, &config(SwitcherTrigger::CapsE, true));
    assert_eq!(host.native_cmd_tab, Some(true));
}

#[test]
fn an_active_remap_is_not_reinstalled_but_its_agent_is_kept_alive() {
    let mut host = granted_host();
    host.remap.active.set(true);
    let _ = Lifecycle::default().start(&mut host, &config(SwitcherTrigger::CapsE, true));
    assert_eq!(*host.remap.calls.borrow(), vec!["ensure_agent"]);
}

#[test]
fn capture_off_removes_an_installed_remap() {
    let mut host = granted_host();
    host.remap.active.set(true);
    let _ = Lifecycle::default().start(&mut host, &config(SwitcherTrigger::CapsE, false));
    assert!(!host.remap.active.get());
    assert_eq!(host.attention, None);
}

#[test]
fn a_failed_install_is_reported_and_flags_the_status_item() {
    let mut host = granted_host();
    host.remap.install_fails = true;
    let _ = Lifecycle::default().start(&mut host, &config(SwitcherTrigger::CapsE, true));
    assert_eq!(host.reports, vec!["caps mapping: hidutil refused"]);
    assert_eq!(host.attention, Some(true));
    assert!(!host.remap.calls.borrow().contains(&"ensure_agent"));
}

#[test]
fn start_without_grant_prompts_restores_cmd_tab_and_polls() {
    let mut host = FakeHost::default();
    let next = Lifecycle::default().start(&mut host, &config(SwitcherTrigger::Both, true));
    assert_eq!(host.prompts, 1);
    assert_eq!(host.native_cmd_tab, Some(true));
    assert_eq!(next, Next::PollGrantAfter(GRANT_POLL_INTERVAL));
    assert_eq!(host.reports, vec!["taps: no Accessibility grant"]);
}

#[test]
fn the_grant_landing_starts_the_taps_and_takes_cmd_tab() {
    let mut host = FakeHost::default();
    let config = config(SwitcherTrigger::Both, true);
    let mut lifecycle = Lifecycle::default();
    let _ = lifecycle.start(&mut host, &config);
    assert_eq!(
        lifecycle.grant_poll(&mut host, &config),
        Next::PollGrantAfter(GRANT_POLL_INTERVAL)
    );
    host.granted = true;
    assert_eq!(lifecycle.grant_poll(&mut host, &config), Next::Settled);
    assert_eq!(host.tap_starts, 2);
    assert_eq!(host.native_cmd_tab, Some(false));
}

#[test]
fn a_grant_still_missing_after_fifteen_seconds_flags_the_status_item_once() {
    let mut host = FakeHost::default();
    let config = config(SwitcherTrigger::CapsE, false);
    let mut lifecycle = Lifecycle::default();
    let _ = lifecycle.start(&mut host, &config);
    let polls_before_stale =
        (Duration::from_secs(15).as_millis() / GRANT_POLL_INTERVAL.as_millis()) as u32 - 1;
    for _ in 0..polls_before_stale {
        let _ = lifecycle.grant_poll(&mut host, &config);
    }
    assert_eq!(host.attention, None);
    let _ = lifecycle.grant_poll(&mut host, &config);
    assert_eq!(host.attention, Some(true));
    host.attention = None;
    let _ = lifecycle.grant_poll(&mut host, &config);
    assert_eq!(host.attention, None);
}

#[test]
fn taps_failing_after_the_grant_are_reported_and_polling_stops() {
    let mut host = FakeHost::default();
    let config = config(SwitcherTrigger::Both, false);
    let mut lifecycle = Lifecycle::default();
    let _ = lifecycle.start(&mut host, &config);
    host.granted = true;
    host.taps_fail = true;
    assert_eq!(lifecycle.grant_poll(&mut host, &config), Next::Settled);
    assert_eq!(host.native_cmd_tab, Some(true));
    assert_eq!(
        host.reports.last().unwrap(),
        "taps after grant: no Accessibility grant"
    );
}

#[test]
fn a_reload_resyncs_cmd_tab_only_while_the_taps_run() {
    let mut idle_host = FakeHost::default();
    let mut idle = Lifecycle::default();
    let _ = idle.start(&mut idle_host, &config(SwitcherTrigger::CapsE, false));
    idle_host.native_cmd_tab = None;
    idle.config_reloaded(&mut idle_host, &config(SwitcherTrigger::CmdTab, false));
    assert_eq!(idle_host.native_cmd_tab, None);

    let mut host = granted_host();
    let mut live = Lifecycle::default();
    let _ = live.start(&mut host, &config(SwitcherTrigger::CapsE, false));
    live.config_reloaded(&mut host, &config(SwitcherTrigger::CmdTab, false));
    assert_eq!(host.native_cmd_tab, Some(false));
}

#[test]
fn turning_capture_on_from_settings_also_keeps_the_agent() {
    let remap = FakeRemap::default();
    set_capture(&remap, true).unwrap();
    assert!(remap.active.get());
    assert!(remap.agent.get());
    set_capture(&remap, false).unwrap();
    assert!(!remap.active.get());
}
