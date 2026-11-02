//! macOS's own Cmd+Tab and Cmd+Shift+Tab switcher, turned off while Mojito
//! owns Cmd+Tab, the way AltTab does it. The setting is window-server state
//! that outlives this process, so `restore_on_exit` turns it back on at quit
//! and on SIGINT or SIGTERM, and every launch syncs it again.

use std::ptr::NonNull;

use block2::RcBlock;
use dispatch2::{_dispatch_source_type_signal, DispatchObject, DispatchQueue, DispatchSource};
use objc2_app_kit::NSApplicationWillTerminateNotification;
use objc2_foundation::{NSNotification, NSNotificationCenter};

const COMMAND_TAB: i32 = 1;
const COMMAND_SHIFT_TAB: i32 = 2;
const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;
const SIG_IGN: usize = 1;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    safe fn CGSSetSymbolicHotKeyEnabled(hotkey: i32, enabled: bool) -> i32;
}

unsafe extern "C" {
    safe fn signal(signum: i32, handler: usize) -> usize;
}

pub fn set_enabled(enabled: bool) {
    for hotkey in [COMMAND_TAB, COMMAND_SHIFT_TAB] {
        let error = CGSSetSymbolicHotKeyEnabled(hotkey, enabled);
        if error != 0 {
            eprintln!("mojito: native switcher hotkey {hotkey} enabled={enabled}: CGError {error}");
        }
    }
}

/// Turn the native switcher back on however the app ends, short of a crash
/// or SIGKILL; the next launch covers those. Call once, on the main thread.
pub fn restore_on_exit() {
    let on_terminate = RcBlock::new(|_: NonNull<NSNotification>| set_enabled(true));
    let center = NSNotificationCenter::defaultCenter();
    let _observer = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSApplicationWillTerminateNotification),
            None,
            None,
            &on_terminate,
        )
    };
    for signum in [SIGINT, SIGTERM] {
        signal(signum, SIG_IGN);
        let source = unsafe {
            DispatchSource::new(
                (&raw const _dispatch_source_type_signal).cast_mut(),
                signum as usize,
                0,
                Some(DispatchQueue::main()),
            )
        };
        let on_signal = RcBlock::new(|| {
            set_enabled(true);
            std::process::exit(0);
        });
        unsafe { source.set_event_handler_with_block(RcBlock::as_ptr(&on_signal)) };
        source.resume();
        std::mem::forget(source);
    }
}
