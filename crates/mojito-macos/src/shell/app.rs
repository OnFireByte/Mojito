use std::time::Duration;

use dispatch2::{DispatchQueue, DispatchTime};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

pub fn become_accessory() {
    NSApplication::sharedApplication(main_thread())
        .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

pub fn run() {
    NSApplication::sharedApplication(main_thread()).run();
}

pub fn terminate() {
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).terminate(None);
    }
}

pub fn on_main(job: impl FnOnce() + Send + 'static) {
    DispatchQueue::main().exec_async(job);
}

pub fn on_main_after(delay: Duration, job: impl FnOnce() + Send + 'static) {
    let when = DispatchTime::try_from(delay).unwrap_or(DispatchTime::NOW);
    if DispatchQueue::main().after(when, job).is_err() {
        eprintln!("mojito: could not schedule main-queue work after {delay:?}");
    }
}

fn main_thread() -> MainThreadMarker {
    MainThreadMarker::new().expect("the app runs on the main thread")
}
