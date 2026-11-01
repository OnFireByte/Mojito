//! Three event taps on one CFRunLoop thread, plus the health timer.
//! Session tap: filtering, keyDown|keyUp only, flagsChanged never filtered.
//! HID tap: listen-only keyDown, evidence when a Carbon hotkey consumes F18.
//! Callbacks never block: pure engine inline, synth posted inline, Ui/Job
//! over channels with a waker poke.

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use mojito_core::bridge::{Job, UiMsg};
use mojito_core::engine::Mono;
use mojito_core::{Action, Engine, ModifierFlags, PhysicalKey, RawKey};

use crate::synth::{is_marked, post};

pub struct TapDeps {
    pub engine: Engine,
    /// Sender for engine Ui actions; a clone of the channel the engine reads
    /// config/index through. Senders live here because the tap thread, not
    /// the pure engine, performs the crossing.
    pub ui: SyncSender<UiMsg>,
    pub jobs: SyncSender<Job>,
    pub ui_wake: Box<dyn Fn() + Send>,
    pub job_wake: Box<dyn Fn() + Send>,
}

// CGEventTypes.h / CGEventSource.h constants (verified against hex).
const EVENT_KEY_DOWN: u32 = 10;
const EVENT_KEY_UP: u32 = 11;
const KEYBOARD_EVENT_KEYCODE: u32 = 9;
const KEYBOARD_EVENT_AUTOREPEAT: u32 = 8;
const TAP_DISABLED_BY_TIMEOUT: u32 = u32::MAX - 1;
const TAP_DISABLED_BY_USER_INPUT: u32 = u32::MAX;
const ANNOTATED_SESSION_EVENT_TAP: u32 = 2;
const HID_EVENT_TAP: u32 = 0;
const HEAD_INSERT_EVENT_TAP: u32 = 0;
const DEFAULT_EVENT_TAP: u32 = 0;
const LISTEN_ONLY_EVENT_TAP: u32 = 1;
const KEY_MASK: u64 = (1_u64 << EVENT_KEY_DOWN) | (1_u64 << EVENT_KEY_UP);
const HID_MASK: u64 = 1_u64 << EVENT_KEY_DOWN;
/// macOS can silently disable or invalidate a tap (sleep, secure input, an
/// Accessibility change); this cadence notices without measurable idle cost.
const HEALTH_INTERVAL_SECONDS: f64 = 5.0;
/// HID-tap timestamps arrive as Mach ticks on Apple Silicon and as
/// nanoseconds for some events; a reading older than this is a misread.
const RECENT_NS: u64 = 5_000_000_000;

type EventRef = *mut c_void;
type EventTapCallback = unsafe extern "C" fn(
    proxy: *mut c_void,
    event_type: u32,
    event: EventRef,
    user_info: *mut c_void,
) -> EventRef;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: EventTapCallback,
        user_info: *mut c_void,
    ) -> EventRef;
    fn CGEventTapEnable(tap: EventRef, enable: bool);
    fn CGEventTapIsEnabled(tap: EventRef) -> u8;
    fn CGEventGetFlags(event: EventRef) -> u64;
    fn CGEventGetTimestamp(event: EventRef) -> u64;
    fn CGEventGetIntegerValueField(event: EventRef, field: u32) -> i64;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: EventRef,
        order: isize,
    ) -> *mut c_void;
    fn CFMachPortIsValid(port: EventRef) -> u8;
    fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopRemoveSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopRun();
    fn CFRunLoopStop(run_loop: *mut c_void);
    fn CFRunLoopTimerCreate(
        allocator: *const c_void,
        fire_date: f64,
        interval: f64,
        flags: usize,
        order: isize,
        callout: unsafe extern "C" fn(timer: *mut c_void, info: *mut c_void),
        context: *mut RunLoopTimerContext,
    ) -> *mut c_void;
    fn CFRunLoopAddTimer(run_loop: *mut c_void, timer: *mut c_void, mode: *const c_void);
    fn CFRunLoopTimerInvalidate(timer: *mut c_void);
    fn CFAbsoluteTimeGetCurrent() -> f64;
    fn CFRelease(value: *const c_void);
    static kCFRunLoopCommonModes: *const c_void;
}

// libSystem symbols; always linked on macOS.
unsafe extern "C" {
    fn mach_absolute_time() -> u64;
    fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
}

#[repr(C)]
struct MachTimebaseInfo {
    numer: u32,
    denom: u32,
}

#[repr(C)]
struct RunLoopTimerContext {
    version: isize,
    info: *mut c_void,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
}

/// The tap thread's whole world. Every callback and the health timer run on
/// this one thread, so thread-locals are sound and keep the engine access
/// free of unsafe pointer plumbing.
struct TapState {
    engine: Option<Engine>,
    ui: Option<SyncSender<UiMsg>>,
    jobs: Option<SyncSender<Job>>,
    ui_wake: Option<Box<dyn Fn() + Send>>,
    job_wake: Option<Box<dyn Fn() + Send>>,
    taps: TapSet,
}

thread_local! {
    static TAP: RefCell<TapState> = RefCell::new(TapState {
        engine: None,
        ui: None,
        jobs: None,
        ui_wake: None,
        job_wake: None,
        taps: TapSet { run_loop: ptr::null_mut(), session: None, hid: None, session_failing: false, hid_failing: false },
    });
}

/// Live while the tap thread's run loop runs; `stop()` stops it from anywhere.
static RUN_LOOP: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static WORKER: Mutex<Option<thread::JoinHandle<()>>> = Mutex::new(None);

/// Spawn the tap thread; readiness is signaled once the run loop is live.
/// Returns an error string on failure so the permission flow can explain it.
/// Calling `spawn` again is legitimate only after `stop` or a failed start
/// (a failed tap creation exits the thread immediately).
pub fn spawn(deps: TapDeps) -> Result<(), String> {
    {
        let worker = WORKER.lock().unwrap_or_else(|error| error.into_inner());
        if worker.as_ref().is_some_and(|handle| !handle.is_finished()) {
            return Err("event taps are already running".into());
        }
    }
    let (ready_tx, ready_rx) = sync_channel(1);
    let handle = thread::Builder::new()
        .name("mojito-tap".into())
        .spawn(move || run(deps, ready_tx))
        .map_err(|error| format!("could not spawn the tap thread: {error}"))?;
    *WORKER.lock().unwrap_or_else(|error| error.into_inner()) = Some(handle);
    match ready_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => Err(message),
        Err(_) => Err("timed out starting the event taps".into()),
    }
}

/// Stop the run loop from any thread (quit discipline). The tap thread then
/// detaches its taps and exits; joining it is the caller's bounded business.
pub fn stop() {
    let run_loop = RUN_LOOP.load(Ordering::Acquire);
    if !run_loop.is_null() {
        // SAFETY: Core Foundation run loops may be stopped from any thread.
        unsafe { CFRunLoopStop(run_loop) };
    }
}

fn run(deps: TapDeps, ready: SyncSender<Result<(), String>>) {
    TAP.with_borrow_mut(|state| {
        state.engine = Some(deps.engine);
        state.ui = Some(deps.ui);
        state.jobs = Some(deps.jobs);
        state.ui_wake = Some(deps.ui_wake);
        state.job_wake = Some(deps.job_wake);
    });

    // SAFETY: This is the dedicated event-tap thread's current run loop.
    let run_loop = unsafe { CFRunLoopGetCurrent() };
    let session = TAP.with_borrow_mut(|state| {
        state.taps.run_loop = run_loop;
        state.taps.attach_session()
    });
    if let Err(message) = session {
        let _ = ready.send(Err(format!(
            "could not create the key event tap ({message}); grant Accessibility permission"
        )));
        return;
    }
    if let Err(message) = TAP.with_borrow_mut(|state| state.taps.attach_hid()) {
        // The HID tap only adds F18 evidence for Carbon-consumed presses;
        // Mojito still works without it.
        eprintln!("mojito-macos: could not create the HID evidence tap: {message}");
    }

    let _health = HealthTimer::start(run_loop);
    let _app_nap = AppNapOptOut::begin();
    RUN_LOOP.store(run_loop, Ordering::Release);
    let _ = ready.send(Ok(()));
    // SAFETY: This dedicated thread exists solely to service the taps.
    unsafe { CFRunLoopRun() };
    RUN_LOOP.store(ptr::null_mut(), Ordering::Release);
    // Thread-locals drop here: taps detach (disable + release), senders close.
}

/// The filtering callback. Marker check first, then RawKey, then the engine.
/// Only keyDown|keyUp arrive: flagsChanged is not in the mask (an active
/// filtering tap there breaks Finder's Option menus even on pass-through).
unsafe extern "C" fn on_key_event(
    _proxy: *mut c_void,
    event_type: u32,
    event: *mut c_void,
    _ctx: *mut c_void,
) -> *mut c_void {
    if matches!(
        event_type,
        TAP_DISABLED_BY_TIMEOUT | TAP_DISABLED_BY_USER_INPUT
    ) {
        TAP.with_borrow_mut(|state| {
            if state.taps.reenable_all()
                && let Some(engine) = &mut state.engine
            {
                for action in engine.reset() {
                    apply(state, action);
                }
            }
        });
        return event;
    }
    if event.is_null() || !matches!(event_type, EVENT_KEY_DOWN | EVENT_KEY_UP) {
        return event;
    }
    if unsafe { is_marked(event) } {
        return event;
    }
    // The UI's click-to-cancel control path: an unmarked keyDown on the
    // reserved control keycode. Cancel through the engine, then swallow the
    // event so no app ever sees it.
    if event_type == EVENT_KEY_DOWN
        && unsafe { CGEventGetIntegerValueField(event, KEYBOARD_EVENT_KEYCODE) }
            == mojito_core::CONTROL.0 as i64
    {
        TAP.with_borrow_mut(|state| {
            if let Some(engine) = &mut state.engine {
                for action in engine.cancel_overlay() {
                    apply(state, action);
                }
            }
        });
        return ptr::null_mut();
    }
    // The settings process's save signal: reload the config on main.
    if event_type == EVENT_KEY_DOWN
        && unsafe { CGEventGetIntegerValueField(event, KEYBOARD_EVENT_KEYCODE) }
            == mojito_core::CONTROL_RELOAD.0 as i64
    {
        TAP.with_borrow_mut(|state| {
            if state.jobs.as_ref().is_some_and(|tx| tx.try_send(mojito_core::Job::ReloadConfig).is_ok())
                && let Some(wake) = state.job_wake.as_ref()
            {
                wake();
            }
        });
        return ptr::null_mut();
    }
    let pass = TAP.with_borrow_mut(|state| {
        let Some(engine) = &mut state.engine else {
            return true;
        };
        let mut pass = true;
        for action in engine.on_event(raw_from(event, event_type)) {
            match action {
                Action::Pass => pass = true,
                Action::Swallow => pass = false,
                other => apply(state, other),
            }
        }
        pass
    });
    if pass { event } else { ptr::null_mut() }
}

/// The listen-only HID callback. Evidence only: a keyDown seen here that no
/// session event will match means a Carbon hotkey consumed it above the
/// session tap, so meta must still engage. Only F18 feeds the engine; every
/// other key arrives through the session tap alone, which keeps one press
/// from being processed twice. Listen-only taps can never swallow.
unsafe extern "C" fn on_hid_evidence(
    _proxy: *mut c_void,
    event_type: u32,
    event: *mut c_void,
    _ctx: *mut c_void,
) -> *mut c_void {
    if matches!(
        event_type,
        TAP_DISABLED_BY_TIMEOUT | TAP_DISABLED_BY_USER_INPUT
    ) {
        TAP.with_borrow_mut(|state| {
            state.taps.reenable_all();
        });
        return event;
    }
    if event.is_null() || event_type != EVENT_KEY_DOWN || unsafe { is_marked(event) } {
        return event;
    }
    // SAFETY: CoreGraphics supplied a valid keyboard event.
    let code = unsafe { CGEventGetIntegerValueField(event, KEYBOARD_EVENT_KEYCODE) } as u16;
    if code != mojito_core::F18.0 {
        return event;
    }
    TAP.with_borrow_mut(|state| {
        let Some(engine) = &mut state.engine else {
            return;
        };
        for action in engine.on_event(raw_from(event, EVENT_KEY_DOWN)) {
            // Swallow/Pass are meaningless to a listen-only tap; only the
            // side-effect actions matter here.
            match action {
                Action::Pass | Action::Swallow => {}
                other => apply(state, other),
            }
        }
    });
    event
}

/// Backend conversion: a CGEvent into the engine's raw input.
fn raw_from(event: *mut c_void, event_type: u32) -> RawKey {
    // SAFETY: CoreGraphics supplied a valid keyboard event to the callback;
    // each call is a read-only field query.
    let code = unsafe { CGEventGetIntegerValueField(event, KEYBOARD_EVENT_KEYCODE) } as u16;
    // SAFETY: as above.
    let repeat = unsafe { CGEventGetIntegerValueField(event, KEYBOARD_EVENT_AUTOREPEAT) != 0 };
    // SAFETY: as above.
    let flags = unsafe { CGEventGetFlags(event) };
    RawKey {
        key: PhysicalKey(code),
        down: event_type == EVENT_KEY_DOWN,
        repeat,
        flags: ModifierFlags(flags),
        t: event_mono(event),
    }
}

/// The event's timestamp mapped onto the monotonic nanosecond timeline the
/// engine's `Mono` lives on. Session events carry nanoseconds; HID events
/// carried raw Mach ticks on Apple Silicon but nanoseconds for some events:
/// sniff which reading is recent and never re-apply a conversion.
fn event_mono(event: *mut c_void) -> Mono {
    // SAFETY: CoreGraphics supplied a valid event; read-only query.
    let raw = unsafe { CGEventGetTimestamp(event) };
    let now = now_ns();
    let as_ticks = ticks_to_ns(raw);
    if as_ticks <= now && now - as_ticks <= RECENT_NS {
        return Mono(as_ticks);
    }
    if raw <= now && now - raw <= RECENT_NS {
        return Mono(raw);
    }
    Mono(now)
}

/// Monotonic nanoseconds since boot (the Mach clock scaled by its timebase).
fn now_ns() -> u64 {
    let ticks = unsafe { mach_absolute_time() };
    ticks_to_ns(ticks)
}

fn ticks_to_ns(ticks: u64) -> u64 {
    static TIMEBASE: OnceLock<(u32, u32)> = OnceLock::new();
    let (numer, denom) = *TIMEBASE.get_or_init(|| {
        // SAFETY: a plain C struct is filled in; a zero denominator falls
        // back to 1:1 rather than dividing by zero below.
        unsafe {
            let mut info = MachTimebaseInfo { numer: 0, denom: 0 };
            let kern = mach_timebase_info(&mut info);
            if kern == 0 && info.denom != 0 {
                (info.numer, info.denom)
            } else {
                (1, 1)
            }
        }
    });
    ((ticks as u128 * numer as u128) / denom as u128) as u64
}

/// Execute one action on the tap thread (synth inline, Ui/Job over channels).
/// `try_send` keeps the callback non-blocking: a full or closed channel drops
/// the message instead of stalling the event stream. Owned, because the
/// bridge messages are not Clone and the engine hands each action out once.
fn apply(state: &mut TapState, action: Action) {
    match action {
        Action::Pass | Action::Swallow => {}
        Action::Synth(key) => post(&key),
        Action::Ui(msg) => {
            eprintln!("mojito: ui {msg:?}"); // TEMP diagnosis
            if state.ui.as_ref().is_some_and(|tx| tx.try_send(msg).is_ok())
                && let Some(wake) = state.ui_wake.as_ref()
            {
                wake();
            }
        }
        Action::Job(job) => {
            if state
                .jobs
                .as_ref()
                .is_some_and(|tx| tx.try_send(job).is_ok())
                && let Some(wake) = state.job_wake.as_ref()
            {
                wake();
            }
        }
    }
}

/// One attached tap plus its run-loop source, detached on drop.
struct Attached {
    tap: EventRef,
    source: *mut c_void,
    run_loop: *mut c_void,
}

impl Attached {
    /// # Safety
    /// The run loop must be the calling thread's, and the returned tap must
    /// only be used on that thread.
    unsafe fn create(
        location: u32,
        options: u32,
        mask: u64,
        callback: EventTapCallback,
        run_loop: *mut c_void,
    ) -> Result<Self, &'static str> {
        // SAFETY: forwarded from the caller; user_info is unused (state is
        // thread-local to the tap thread).
        let tap = unsafe {
            CGEventTapCreate(
                location,
                HEAD_INSERT_EVENT_TAP,
                options,
                mask,
                callback,
                ptr::null_mut(),
            )
        };
        if tap.is_null() {
            return Err("CGEventTapCreate returned null");
        }
        // SAFETY: `tap` is a valid CFMachPort from CGEventTapCreate.
        let source = unsafe { CFMachPortCreateRunLoopSource(ptr::null(), tap, 0) };
        if source.is_null() {
            // SAFETY: owned tap is released here.
            unsafe { CFRelease(tap) };
            return Err("could not create the run-loop source");
        }
        // SAFETY: `source` and `run_loop` are valid for this thread.
        unsafe {
            CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
            CGEventTapEnable(tap, true);
        }
        Ok(Self {
            tap,
            source,
            run_loop,
        })
    }
}

impl Drop for Attached {
    fn drop(&mut self) {
        // SAFETY: this is the final owner of both CF objects, and dropping
        // happens on the tap thread that added the source.
        unsafe {
            CGEventTapEnable(self.tap, false);
            CFRunLoopRemoveSource(self.run_loop, self.source, kCFRunLoopCommonModes);
            CFRelease(self.source);
            CFRelease(self.tap);
        }
    }
}

/// The taps on the event-tap thread, checked by its health timer.
struct TapSet {
    run_loop: *mut c_void,
    session: Option<Attached>,
    hid: Option<Attached>,
    /// Creation failed and was logged; log again only once it recovers.
    session_failing: bool,
    hid_failing: bool,
}

impl TapSet {
    fn attach_session(&mut self) -> Result<(), &'static str> {
        // SAFETY: the run loop is the calling thread's; the state outlives
        // the tap.
        let attached = unsafe {
            Attached::create(
                ANNOTATED_SESSION_EVENT_TAP,
                DEFAULT_EVENT_TAP,
                KEY_MASK,
                on_key_event,
                self.run_loop,
            )
        }?;
        self.session = Some(attached);
        Ok(())
    }

    fn attach_hid(&mut self) -> Result<(), &'static str> {
        // SAFETY: as `attach_session`.
        let attached = unsafe {
            Attached::create(
                HID_EVENT_TAP,
                LISTEN_ONLY_EVENT_TAP,
                HID_MASK,
                on_hid_evidence,
                self.run_loop,
            )
        }?;
        self.hid = Some(attached);
        Ok(())
    }

    /// Re-enable all taps after macOS disabled one without notice.
    fn reenable_all(&mut self) -> bool {
        let mut reenabled = false;
        for attached in [&mut self.session, &mut self.hid].into_iter().flatten() {
            // SAFETY: `tap` is a live CFMachPort owned by `Attached`.
            unsafe { CGEventTapEnable(attached.tap, true) };
            reenabled = true;
        }
        reenabled
    }

    /// Re-enable a disabled tap, recreate an invalidated or missing one.
    /// Returns whether the session tap's coverage was interrupted, which
    /// makes the engine's state untrustworthy and calls for a reset. The HID
    /// tap only adds evidence, so a gap there does not.
    fn check(&mut self) -> bool {
        let mut interrupted = false;
        interrupted |= self.maintain_tap(true);
        self.maintain_tap(false);
        interrupted
    }

    fn maintain_tap(&mut self, session: bool) -> bool {
        let (location, options, mask, callback) = if session {
            (
                ANNOTATED_SESSION_EVENT_TAP,
                DEFAULT_EVENT_TAP,
                KEY_MASK,
                on_key_event as EventTapCallback,
            )
        } else {
            (
                HID_EVENT_TAP,
                LISTEN_ONLY_EVENT_TAP,
                HID_MASK,
                on_hid_evidence as EventTapCallback,
            )
        };
        let slot = if session {
            &mut self.session
        } else {
            &mut self.hid
        };
        let mut interrupted = false;
        let recreate = match slot {
            None => true,
            Some(attached) => {
                // SAFETY: `tap` is a live CFMachPort owned by `Attached`.
                if unsafe { CFMachPortIsValid(attached.tap) } == 0 {
                    true
                } else if unsafe { CGEventTapIsEnabled(attached.tap) } == 0 {
                    // SAFETY: as above.
                    unsafe { CGEventTapEnable(attached.tap, true) };
                    // SAFETY: as above.
                    let enabled = unsafe { CGEventTapIsEnabled(attached.tap) } != 0;
                    interrupted = session;
                    !enabled
                } else {
                    false
                }
            }
        };
        if recreate {
            let was_attached = slot.take().is_some();
            interrupted |= was_attached && session;
            // SAFETY: the run loop is the calling thread's and the state
            // outlives the tap.
            match unsafe { Attached::create(location, options, mask, callback, self.run_loop) } {
                Ok(attached) => {
                    if was_attached {
                        eprintln!(
                            "mojito-macos: recreated the {} event tap",
                            kind_name(session)
                        );
                    }
                    if session {
                        self.session_failing = false;
                    } else {
                        self.hid_failing = false;
                    }
                    *slot = Some(attached);
                }
                Err(message) => {
                    let already_failing = if session {
                        self.session_failing
                    } else {
                        self.hid_failing
                    };
                    if !already_failing {
                        eprintln!(
                            "mojito-macos: could not recreate the {} event tap: {message}",
                            kind_name(session)
                        );
                    }
                    if session {
                        self.session_failing = true;
                    } else {
                        self.hid_failing = true;
                    }
                }
            }
        } else if session {
            self.session_failing = false;
        } else {
            self.hid_failing = false;
        }
        interrupted
    }
}

fn kind_name(session: bool) -> &'static str {
    if session { "session" } else { "hid" }
}

/// The repeating run-loop timer that runs `TapSet::check`.
struct HealthTimer(*mut c_void);

impl HealthTimer {
    fn start(run_loop: *mut c_void) -> Option<Self> {
        let mut context = RunLoopTimerContext {
            version: 0,
            info: ptr::null_mut(),
            retain: ptr::null(),
            release: ptr::null(),
            copy_description: ptr::null(),
        };
        // SAFETY: CoreFoundation copies the context; the callout only touches
        // thread-locals, so `info` can stay null.
        let timer = unsafe {
            CFRunLoopTimerCreate(
                ptr::null(),
                CFAbsoluteTimeGetCurrent() + HEALTH_INTERVAL_SECONDS,
                HEALTH_INTERVAL_SECONDS,
                0,
                0,
                check_tap_health,
                &raw mut context,
            )
        };
        if timer.is_null() {
            eprintln!("mojito-macos: could not schedule event tap health checks");
            return None;
        }
        // SAFETY: `timer` is a valid run-loop timer for this thread's loop.
        unsafe { CFRunLoopAddTimer(run_loop, timer, kCFRunLoopCommonModes) };
        Some(Self(timer))
    }
}

impl Drop for HealthTimer {
    fn drop(&mut self) {
        // SAFETY: the timer was created by `HealthTimer::start` and is owned here.
        unsafe {
            CFRunLoopTimerInvalidate(self.0);
            CFRelease(self.0);
        }
    }
}

unsafe extern "C" fn check_tap_health(_timer: *mut c_void, _info: *mut c_void) {
    TAP.with_borrow_mut(|state| {
        if state.taps.check()
            && let Some(engine) = &mut state.engine
        {
            for action in engine.reset() {
                apply(state, action);
            }
        }
    });
}

/// Keeps App Nap from throttling the tap thread while Mojito sits idle in
/// the background: a napped callback can miss its deadline and the health
/// timer would stretch. Idle system sleep stays allowed.
struct AppNapOptOut(
    objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2::runtime::NSObjectProtocol>>,
);

impl AppNapOptOut {
    fn begin() -> Self {
        use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
        let activity = NSProcessInfo::processInfo().beginActivityWithOptions_reason(
            NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
            &NSString::from_str("Mojito listens for its Caps Lock shortcut"),
        );
        Self(activity)
    }
}

impl Drop for AppNapOptOut {
    fn drop(&mut self) {
        // SAFETY: the activity came from `beginActivityWithOptions:reason:`.
        unsafe { objc2_foundation::NSProcessInfo::processInfo().endActivity(&self.0) };
    }
}
