use std::time::Duration;

use crate::config::Config;

pub const GRANT_POLL_INTERVAL: Duration = Duration::from_millis(250);
const GRANT_STALE_AFTER: Duration = Duration::from_secs(15);

pub trait Remap {
    fn is_active(&self) -> bool;
    fn install(&self) -> Result<(), String>;
    fn uninstall(&self) -> Result<(), String>;
    fn ensure_agent(&self) -> Result<(), String>;
}

pub trait Host {
    fn remap(&self) -> &dyn Remap;
    fn grant_ready(&self) -> bool;
    fn prompt_for_grant(&mut self);
    fn start_taps(&mut self) -> Result<(), String>;
    fn set_native_cmd_tab(&mut self, enabled: bool);
    fn set_needs_attention(&mut self, flag: bool);
    fn report(&mut self, context: &str, error: &str);
}

#[must_use]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Next {
    Settled,
    PollGrantAfter(Duration),
}

#[derive(Default, Debug)]
pub struct Lifecycle {
    taps_live: bool,
    waited_for_grant: Duration,
    flagged_stale_grant: bool,
}

impl Lifecycle {
    pub fn start(&mut self, host: &mut impl Host, config: &Config) -> Next {
        sync_capture(host, config);
        if !host.grant_ready() {
            host.prompt_for_grant();
        }
        match host.start_taps() {
            Ok(()) => {
                self.go_live(host, config);
                Next::Settled
            }
            Err(error) => {
                host.report("taps", &error);
                host.set_native_cmd_tab(true);
                Next::PollGrantAfter(GRANT_POLL_INTERVAL)
            }
        }
    }

    pub fn grant_poll(&mut self, host: &mut impl Host, config: &Config) -> Next {
        if !host.grant_ready() {
            self.waited_for_grant += GRANT_POLL_INTERVAL;
            if self.waited_for_grant >= GRANT_STALE_AFTER && !self.flagged_stale_grant {
                self.flagged_stale_grant = true;
                host.set_needs_attention(true);
            }
            return Next::PollGrantAfter(GRANT_POLL_INTERVAL);
        }
        match host.start_taps() {
            Ok(()) => self.go_live(host, config),
            Err(error) => host.report("taps after grant", &error),
        }
        Next::Settled
    }

    pub fn config_reloaded(&mut self, host: &mut impl Host, config: &Config) {
        if self.taps_live {
            sync_native_cmd_tab(host, config);
        }
    }

    fn go_live(&mut self, host: &mut impl Host, config: &Config) {
        self.taps_live = true;
        sync_native_cmd_tab(host, config);
    }
}

pub fn set_capture(remap: &dyn Remap, on: bool) -> Result<(), String> {
    if on {
        if !remap.is_active() {
            remap.install()?;
        }
        remap.ensure_agent()
    } else if remap.is_active() {
        remap.uninstall()
    } else {
        Ok(())
    }
}

fn sync_capture(host: &mut impl Host, config: &Config) {
    if let Err(error) = set_capture(host.remap(), config.capture_enabled) {
        host.report("caps mapping", &error);
    }
    if config.capture_enabled && !host.remap().is_active() {
        host.set_needs_attention(true);
    }
}

fn sync_native_cmd_tab(host: &mut impl Host, config: &Config) {
    host.set_native_cmd_tab(!config.switcher_trigger.cmd_tab());
}
