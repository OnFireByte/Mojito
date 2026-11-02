//! Event taps on one CFRunLoop thread, plus the health timer. Three filter
//! taps share one callback, one per public tap point: the HID-head tap sees
//! hardware keys first (another app's HID-level key filter — iTerm2's
//! hotkey window — otherwise consumes keys before any session tap sees
//! them), the session tap sits ahead of the system's own session filters
//! (Siri's key filters), and the annotated-session tap catches what enters
//! after annotation. An injected key (Touch-Bar-style Esc, driver-synthesized
//! keys) can enter at any of the three points; together they cover every
//! path a key can take to an app. The engine deduplicates the multiple
//! deliveries by event identity. flagsChanged is never filtered (an active
//! filtering tap there breaks Finder's Option menus even on pass-through).
//! Callbacks never block: pure engine inline, synth posted inline, Ui/Job
//! over channels with a waker poke.

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::{self, NonNull};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use block2::RcBlock;
use mojito_core::bridge::{Job, UiMsg};
use mojito_core::engine::Mono;
use mojito_core::{Action, Engine, PhysicalKey, RawKey};
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_core_foundation::{
    CFAbsoluteTimeGetCurrent, CFMachPort, CFRetained, CFRunLoop, CFRunLoopMode, CFRunLoopSource,
    CFRunLoopTimer, Type, kCFRunLoopCommonModes,
};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventMask, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventTapProxy, CGEventType,
};
use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};

use super::synth::{is_marked, post};
use crate::{abort_on_panic, lock};

pub struct TapDeps {
    pub engine: Engine,
    /// Sender for engine Ui actions; a clone of the channel the engine reads
    /// config/index through. Senders live here because the tap thread, not
    /// the pure engine, performs the crossing.
    pub ui: SyncSender<UiMsg>,
    pub jobs: SyncSender<Job>,
    pub wake: Box<dyn Fn() + Send>,
}

const KEY_MASK: CGEventMask = mask_of(CGEventType::KeyDown) | mask_of(CGEventType::KeyUp);
/// macOS can silently disable or invalidate a tap (sleep, secure input, an
/// Accessibility change); this cadence notices without measurable idle cost.
const HEALTH_INTERVAL_SECONDS: f64 = 5.0;
/// HID-tap timestamps arrive as Mach ticks on Apple Silicon and as
/// nanoseconds for some events; a reading older than this is a misread.
const RECENT_NS: u64 = 5_000_000_000;

const fn mask_of(event_type: CGEventType) -> CGEventMask {
    1 << event_type.0
}

struct TapSpec {
    name: &'static str,
    location: CGEventTapLocation,
    required: bool,
}

const TAP_SPECS: [TapSpec; 3] = [
    TapSpec {
        name: "HID",
        location: CGEventTapLocation::HIDEventTap,
        required: true,
    },
    TapSpec {
        name: "session",
        location: CGEventTapLocation::SessionEventTap,
        required: false,
    },
    TapSpec {
        name: "annotated session",
        location: CGEventTapLocation::AnnotatedSessionEventTap,
        required: false,
    },
];

/// The tap thread's whole world. Every callback and the health timer run on
/// this one thread, so thread-locals are sound and keep the engine access
/// free of unsafe pointer plumbing.
struct TapThread {
    deps: TapDeps,
    taps: Vec<Tap>,
}

thread_local! {
    static TAP_THREAD: RefCell<Option<TapThread>> = const { RefCell::new(None) };
}

fn with_tap_thread<R>(f: impl FnOnce(&mut TapThread) -> R) -> Option<R> {
    TAP_THREAD.with_borrow_mut(|state| state.as_mut().map(f))
}

/// Live while the tap thread's run loop runs; `stop()` stops it from anywhere.
static RUN_LOOP: Mutex<Option<StoppableRunLoop>> = Mutex::new(None);
static WORKER: Mutex<Option<thread::JoinHandle<()>>> = Mutex::new(None);

/// The tap thread's run loop, retained so `stop()` can never reach a freed one.
struct StoppableRunLoop(CFRetained<CFRunLoop>);

// SAFETY: the only use from another thread is `CFRunLoopStop`, which
// CoreFoundation documents as callable from any thread; the retain keeps the
// run loop alive however long the handle is held.
unsafe impl Send for StoppableRunLoop {}

/// Spawn the tap thread; readiness is signaled once the run loop is live.
/// Returns an error string on failure so the permission flow can explain it.
/// Calling `spawn` again is legitimate only after `stop` or a failed start
/// (a failed tap creation exits the thread immediately).
pub fn spawn(deps: TapDeps) -> Result<(), String> {
    let mut worker = lock(&WORKER);
    if worker.as_ref().is_some_and(|handle| !handle.is_finished()) {
        return Err("event taps are already running".into());
    }
    let (ready_tx, ready_rx) = sync_channel(1);
    let handle = thread::Builder::new()
        .name("mojito-tap".into())
        .spawn(move || run(deps, ready_tx))
        .map_err(|error| format!("could not spawn the tap thread: {error}"))?;
    *worker = Some(handle);
    drop(worker);
    match ready_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(result) => result,
        Err(_) => Err("timed out starting the event taps".into()),
    }
}

/// Stop the run loop from any thread (quit discipline). The tap thread then
/// detaches its taps and exits; joining it is the caller's bounded business.
pub fn stop() {
    if let Some(run_loop) = lock(&RUN_LOOP).as_ref() {
        run_loop.0.stop();
    }
}

fn run(deps: TapDeps, ready: SyncSender<Result<(), String>>) {
    let Some(run_loop) = CFRunLoop::current() else {
        let _ = ready.send(Err("the tap thread has no run loop".into()));
        return;
    };
    let mut taps = Vec::with_capacity(TAP_SPECS.len());
    for spec in &TAP_SPECS {
        let attached = Attached::create(spec, &run_loop);
        match &attached {
            Err(message) if spec.required => {
                let _ = ready.send(Err(format!(
                    "could not create the key event tap ({message}); grant Accessibility permission"
                )));
                return;
            }
            Err(message) => {
                eprintln!(
                    "mojito-macos: could not create the {} tap: {message}",
                    spec.name
                )
            }
            Ok(_) => {}
        }
        taps.push(Tap {
            spec,
            attached: attached.ok(),
            failing: false,
        });
    }
    TAP_THREAD.set(Some(TapThread { deps, taps }));

    let _health = HealthTimer::start(&run_loop);
    let _app_nap = AppNapOptOut::begin();
    *lock(&RUN_LOOP) = Some(StoppableRunLoop(run_loop));
    let _ = ready.send(Ok(()));
    CFRunLoop::run();
    lock(&RUN_LOOP).take();
    TAP_THREAD.take();
}

unsafe extern "C-unwind" fn on_key_event(
    _proxy: CGEventTapProxy,
    event_type: CGEventType,
    event: NonNull<CGEvent>,
    _user_info: *mut c_void,
) -> *mut CGEvent {
    // SAFETY: CoreGraphics hands the callback a live event that it owns for
    // the duration of the call; the reference does not escape this call.
    let event_ref = unsafe { event.as_ref() };
    let pass = abort_on_panic(|| handle_key_event(event_type, event_ref));
    if pass {
        event.as_ptr()
    } else {
        ptr::null_mut()
    }
}

fn handle_key_event(event_type: CGEventType, event: &CGEvent) -> bool {
    if matches!(
        event_type,
        CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
    ) {
        with_tap_thread(|tap| {
            if tap.reenable_all() {
                tap.reset_engine();
            }
        });
        return true;
    }
    let is_down = event_type == CGEventType::KeyDown;
    if !(is_down || event_type == CGEventType::KeyUp) || is_marked(event) {
        return true;
    }
    let raw = raw_from(event, is_down);
    with_tap_thread(|tap| {
        if is_down && raw.key == super::synth::CONTROL {
            // The UI's click-to-cancel control path: cancel through the
            // engine, then swallow the event so no app ever sees it.
            let actions = tap.deps.engine.cancel_overlay();
            tap.perform(actions);
            return false;
        }
        if is_down && raw.key == super::synth::CONTROL_RELOAD {
            // The settings process's save signal: reload the config on main.
            tap.send_job(Job::ReloadConfig);
            return false;
        }
        let actions = tap.deps.engine.on_event(raw);
        tap.perform(actions)
    })
    .unwrap_or(true)
}

fn raw_from(event: &CGEvent, down: bool) -> RawKey {
    let field = |field| CGEvent::integer_value_field(Some(event), field);
    RawKey {
        key: PhysicalKey(field(CGEventField::KeyboardEventKeycode) as u16),
        down,
        repeat: field(CGEventField::KeyboardEventAutorepeat) != 0,
        flags: super::modifiers::from_cg(CGEvent::flags(Some(event))),
        t: event_mono(CGEvent::timestamp(Some(event))),
    }
}

/// The event's timestamp mapped onto the monotonic nanosecond timeline the
/// engine's `Mono` lives on. Session events carry nanoseconds; HID events
/// carried raw Mach ticks on Apple Silicon but nanoseconds for some events:
/// sniff which reading is recent and never re-apply a conversion.
fn event_mono(raw: u64) -> Mono {
    let now = now_ns();
    let is_recent = |ns: u64| ns <= now && now - ns <= RECENT_NS;
    let as_ticks = ticks_to_ns(raw);
    if is_recent(as_ticks) {
        Mono(as_ticks)
    } else if is_recent(raw) {
        Mono(raw)
    } else {
        Mono(now)
    }
}

#[repr(C)]
struct MachTimebaseInfo {
    numer: u32,
    denom: u32,
}

// SAFETY: libSystem's Mach clock functions; neither has preconditions beyond
// a valid out-pointer, which the `&mut` signature guarantees.
unsafe extern "C" {
    safe fn mach_absolute_time() -> u64;
    safe fn mach_timebase_info(info: &mut MachTimebaseInfo) -> i32;
}

fn now_ns() -> u64 {
    ticks_to_ns(mach_absolute_time())
}

fn ticks_to_ns(ticks: u64) -> u64 {
    static TIMEBASE: OnceLock<(u32, u32)> = OnceLock::new();
    let (numer, denom) = *TIMEBASE.get_or_init(|| {
        let mut info = MachTimebaseInfo { numer: 0, denom: 0 };
        if mach_timebase_info(&mut info) == 0 && info.denom != 0 {
            (info.numer, info.denom)
        } else {
            (1, 1)
        }
    });
    ((ticks as u128 * numer as u128) / denom as u128) as u64
}

impl TapThread {
    /// Execute the engine's actions on the tap thread (synth inline, Ui/Job
    /// over channels). Returns whether the triggering event passes through;
    /// the last Pass/Swallow decides.
    fn perform(&mut self, actions: Vec<Action>) -> bool {
        let mut pass = true;
        for action in actions {
            match action {
                Action::Pass => pass = true,
                Action::Swallow => pass = false,
                Action::Synth(key) => post(&key),
                Action::Ui(msg) => send_and_wake(&self.deps.ui, msg, &self.deps.wake),
                Action::Job(job) => self.send_job(job),
            }
        }
        pass
    }

    fn send_job(&self, job: Job) {
        send_and_wake(&self.deps.jobs, job, &self.deps.wake);
    }

    fn reset_engine(&mut self) {
        let actions = self.deps.engine.reset();
        self.perform(actions);
    }

    fn reenable_all(&mut self) -> bool {
        let mut reenabled = false;
        for attached in self.taps.iter().filter_map(|tap| tap.attached.as_ref()) {
            CGEvent::tap_enable(&attached.port, true);
            reenabled = true;
        }
        reenabled
    }

    /// Re-enable disabled taps, recreate invalidated or missing ones; a tap
    /// whose coverage was interrupted makes the engine state untrustworthy,
    /// so the engine resets.
    fn check_health(&mut self) {
        let run_loop = CFRunLoop::current();
        let Some(run_loop) = run_loop.as_deref() else {
            return;
        };
        let interrupted = self
            .taps
            .iter_mut()
            .fold(false, |any, tap| tap.maintain(run_loop) | any);
        if interrupted {
            self.reset_engine();
        }
    }
}

/// `try_send` keeps the callback non-blocking: a full or closed channel drops
/// the message instead of stalling the event stream.
fn send_and_wake<T>(tx: &SyncSender<T>, msg: T, wake: &dyn Fn()) {
    if tx.try_send(msg).is_ok() {
        wake();
    }
}

struct Tap {
    spec: &'static TapSpec,
    attached: Option<Attached>,
    /// Creation failed and was logged; log again only once it recovers.
    failing: bool,
}

impl Tap {
    /// Keep one tap alive: re-enable a disabled tap, recreate an invalidated
    /// or missing one. Returns whether its coverage was interrupted.
    fn maintain(&mut self, run_loop: &CFRunLoop) -> bool {
        let (recreate, mut interrupted) = match &self.attached {
            None => (true, false),
            Some(attached) if !attached.port.is_valid() => (true, false),
            Some(attached) if !CGEvent::tap_is_enabled(&attached.port) => {
                CGEvent::tap_enable(&attached.port, true);
                (!CGEvent::tap_is_enabled(&attached.port), true)
            }
            Some(_) => (false, false),
        };
        if !recreate {
            self.failing = false;
            return interrupted;
        }
        let was_attached = self.attached.take().is_some();
        interrupted |= was_attached;
        match Attached::create(self.spec, run_loop) {
            Ok(attached) => {
                if was_attached {
                    eprintln!("mojito-macos: recreated the {} event tap", self.spec.name);
                }
                self.failing = false;
                self.attached = Some(attached);
            }
            Err(message) => {
                if !self.failing {
                    eprintln!(
                        "mojito-macos: could not recreate the {} event tap: {message}",
                        self.spec.name
                    );
                }
                self.failing = true;
            }
        }
        interrupted
    }
}

struct Attached {
    port: CFRetained<CFMachPort>,
    source: CFRetained<CFRunLoopSource>,
    run_loop: CFRetained<CFRunLoop>,
}

impl Attached {
    fn create(spec: &TapSpec, run_loop: &CFRunLoop) -> Result<Self, &'static str> {
        // SAFETY: the callback matches CGEventTapCallBack, aborts instead of
        // unwinding into CoreGraphics, and ignores `user_info`, so null is a
        // valid value; all its state is in the tap thread's thread-local.
        let port = unsafe {
            CGEvent::tap_create(
                spec.location,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::Default,
                KEY_MASK,
                Some(on_key_event),
                ptr::null_mut(),
            )
        }
        .ok_or("CGEventTapCreate returned null")?;
        let source = CFMachPort::new_run_loop_source(None, Some(&port), 0)
            .ok_or("could not create the run-loop source")?;
        run_loop.add_source(Some(&source), common_modes());
        CGEvent::tap_enable(&port, true);
        Ok(Self {
            port,
            source,
            run_loop: run_loop.retain(),
        })
    }
}

impl Drop for Attached {
    fn drop(&mut self) {
        CGEvent::tap_enable(&self.port, false);
        self.run_loop
            .remove_source(Some(&self.source), common_modes());
    }
}

fn common_modes() -> Option<&'static CFRunLoopMode> {
    // SAFETY: kCFRunLoopCommonModes is an immutable constant exported by
    // CoreFoundation and initialized before any Rust code runs.
    unsafe { kCFRunLoopCommonModes }
}

struct HealthTimer(CFRetained<CFRunLoopTimer>);

impl HealthTimer {
    fn start(run_loop: &CFRunLoop) -> Option<Self> {
        let block = RcBlock::new(|_timer| {
            abort_on_panic(|| with_tap_thread(TapThread::check_health));
        });
        // SAFETY: a `None` allocator selects the default allocator, which
        // CFRunLoopTimerCreateWithHandler accepts; the block is non-null and
        // copied by CoreFoundation.
        let timer = unsafe {
            CFRunLoopTimer::with_handler(
                None,
                CFAbsoluteTimeGetCurrent() + HEALTH_INTERVAL_SECONDS,
                HEALTH_INTERVAL_SECONDS,
                0,
                0,
                Some(&block),
            )
        };
        let Some(timer) = timer else {
            eprintln!("mojito-macos: could not schedule event tap health checks");
            return None;
        };
        run_loop.add_timer(Some(&timer), common_modes());
        Some(Self(timer))
    }
}

impl Drop for HealthTimer {
    fn drop(&mut self) {
        self.0.invalidate();
    }
}

/// Keeps App Nap from throttling the tap thread while Mojito sits idle in
/// the background: a napped callback can miss its deadline and the health
/// timer would stretch. Idle system sleep stays allowed.
struct AppNapOptOut(Retained<ProtocolObject<dyn NSObjectProtocol>>);

impl AppNapOptOut {
    fn begin() -> Self {
        let activity = NSProcessInfo::processInfo().beginActivityWithOptions_reason(
            NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
            &NSString::from_str("Mojito listens for its Caps Lock shortcut"),
        );
        Self(activity)
    }
}

impl Drop for AppNapOptOut {
    fn drop(&mut self) {
        // SAFETY: the activity token came from `beginActivityWithOptions:reason:`
        // and is ended exactly once, here.
        unsafe { NSProcessInfo::processInfo().endActivity(&self.0) };
    }
}
