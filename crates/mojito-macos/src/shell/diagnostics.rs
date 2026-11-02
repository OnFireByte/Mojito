use objc2_core_graphics::{
    CGEventTapInformation, CGEventTapLocation, CGEventTapOptions, CGGetEventTapList,
    CGWindowListOption,
};

use crate::apps::index::window_infos;

/// Debug: every event tap installed on the system, with its pid, tap point,
/// and mask, so a tap that eats keys before ours can be named.
/// `CGGetEventTapList`.
pub fn taps() {
    const MAX: usize = 64;
    // SAFETY: CGEventTapInformation is plain old data (integers, floats and a
    // bool), for which all-zero bytes are a valid value.
    let mut taps: [CGEventTapInformation; MAX] = unsafe { std::mem::zeroed() };
    let mut count = 0_u32;
    // SAFETY: the buffer holds `MAX` entries and CoreGraphics writes at most
    // that many; `count` is a live local.
    let status = unsafe { CGGetEventTapList(MAX as u32, taps.as_mut_ptr(), &mut count) };
    if status.0 != 0 {
        println!("CGGetEventTapList failed");
        return;
    }
    for tap in &taps[..(count as usize).min(MAX)] {
        let owner = if tap.tappingProcess == 0 {
            "kernel".to_string()
        } else {
            process_name(tap.tappingProcess)
        };
        let point = match tap.tapPoint {
            CGEventTapLocation::HIDEventTap => "HID",
            CGEventTapLocation::SessionEventTap => "session",
            CGEventTapLocation::AnnotatedSessionEventTap => "annotated session",
            _ => "?",
        };
        let kind = if tap.options == CGEventTapOptions::Default {
            "filter"
        } else {
            "listen"
        };
        println!(
            "pid {} ({owner}) {point} {kind} mask {:#x} enabled {}",
            tap.tappingProcess, tap.eventsOfInterest, tap.enabled
        );
    }
}

/// Debug: status-level windows (the overlay) and Mojito's own windows.
pub fn windows() {
    let Some(windows) = window_infos(CGWindowListOption::OptionAll) else {
        println!("no window list");
        return;
    };
    let mut printed = 0;
    for window in windows {
        let Some(owner) = window.owner_name else {
            continue;
        };
        let layer = window.layer.unwrap_or(0);
        if !owner.to_lowercase().contains("moji") && layer < 20 {
            continue;
        }
        let number = window.number.unwrap_or(0);
        let alpha = window.alpha.map_or(-1, |alpha| alpha as i64);
        println!("owner {owner:?} window {number} layer {layer} alpha {alpha}");
        printed += 1;
    }
    if printed == 0 {
        println!("no mojito windows on screen");
    }
}

// SAFETY: libproc's `proc_name` writes at most `size` bytes into `buffer`.
unsafe extern "C" {
    fn proc_name(pid: i32, buffer: *mut u8, size: u32) -> i32;
}

fn process_name(pid: i32) -> String {
    let mut buffer = [0_u8; 128];
    // SAFETY: the buffer outlives the call and its length bounds the write.
    let length = unsafe { proc_name(pid, buffer.as_mut_ptr(), buffer.len() as u32) };
    if length <= 0 {
        return "(unknown)".into();
    }
    String::from_utf8_lossy(&buffer[..length as usize]).into_owned()
}
