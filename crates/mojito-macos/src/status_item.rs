//! NSStatusItem via objc2 (hex macos_rune/status_item.rs pattern). Menu:
//! Settings, Start at Login, Restore Caps Lock, About, Quit. Actions cross
//! to the main pump over a SyncSender.

use std::cell::RefCell;
use std::sync::mpsc::SyncSender;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{AnyThread, DefinedClass, msg_send, sel};
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSBitmapImageRep, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem};
use objc2_foundation::{NSData, NSSize, NSString};

/// The drink as menu-bar sprites: the 64px art nearest-decimated by
/// scripts/make-icon.swift to the 1x/2x sizes, kept in-binary so the item
/// works before and without the bundle.
const STATUS_PNG_16: &[u8] = include_bytes!("../../../app/assets/mojito-status-16.png");
const STATUS_PNG_32: &[u8] = include_bytes!("../../../app/assets/mojito-status-32.png");

/// The status item image: the 32px sprite plus the 16px one as the 1x
/// representation, drawn at 16pt. None if AppKit cannot decode the embedded
/// PNGs, which would mean a build missing the generated assets.
fn drink_image() -> Option<Retained<NSImage>> {
    let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(STATUS_PNG_32))?;
    let small = NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(STATUS_PNG_16))?;
    image.addRepresentation(&small);
    image.setSize(NSSize::new(16.0, 16.0));
    Some(image)
}

pub enum MenuAction {
    OpenSettings,
    ToggleLogin,
    RestoreCaps,
    Quit,
}

#[derive(Debug)]
struct MenuTargetIvars {
    actions: SyncSender<MenuAction>,
}

objc2::define_class!(
    #[unsafe(super = objc2_foundation::NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = MenuTargetIvars]
    struct MenuTarget;

    unsafe impl NSObjectProtocol for MenuTarget {}

    impl MenuTarget {
        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: &AnyObject) {
            let _ = self.ivars().actions.try_send(MenuAction::OpenSettings);
        }

        #[unsafe(method(toggleLogin:))]
        fn toggle_login(&self, _sender: &AnyObject) {
            let _ = self.ivars().actions.try_send(MenuAction::ToggleLogin);
        }

        #[unsafe(method(restoreCaps:))]
        fn restore_caps(&self, _sender: &AnyObject) {
            let _ = self.ivars().actions.try_send(MenuAction::RestoreCaps);
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: &AnyObject) {
            let _ = self.ivars().actions.try_send(MenuAction::Quit);
        }
    }
);

impl MenuTarget {
    fn new(actions: SyncSender<MenuAction>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(MenuTargetIvars { actions });
        // SAFETY: the standard NSObject init; the class has no init override.
        unsafe { msg_send![super(this), init] }
    }
}

struct StatusItemController {
    item: Retained<NSStatusItem>,
    /// NSMenuItem.target is a weak reference; the menu items would be left
    /// aiming at a deallocated target if we did not own it here. Never
    /// read: holding it is the point.
    #[allow(dead_code)]
    target: Retained<MenuTarget>,
}

thread_local! {
    /// Main thread only; present once the status item is installed.
    static STATUS_ITEM: RefCell<Option<StatusItemController>> = const { RefCell::new(None) };
}

/// Install the status item; actions cross to the main pump over a channel.
pub fn install(tx: SyncSender<MenuAction>) -> Result<(), String> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| "the status item requires the main thread".to_string())?;
    if STATUS_ITEM.with_borrow(|slot| slot.is_some()) {
        return Err("status item is already installed".into());
    }
    let target = MenuTarget::new(tx, mtm);
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Mojito"));
    menu.setAutoenablesItems(false);
    add_item(&menu, &target, "Settings…", sel!(openSettings:), mtm);
    add_item(&menu, &target, "Start at Login", sel!(toggleLogin:), mtm);
    add_item(&menu, &target, "Restore Caps Lock", sel!(restoreCaps:), mtm);
    // About has no engine action; show it plain and disabled.
    let about = plain_item("About Mojito", mtm);
    about.setEnabled(false);
    menu.addItem(&about);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    add_item(&menu, &target, "Quit Mojito", sel!(quit:), mtm);

    let item = NSStatusBar::systemStatusBar().statusItemWithLength(-2.0);
    let button = item
        .button(mtm)
        .ok_or_else(|| "status item button is unavailable".to_string())?;
    let Some(image) = drink_image() else {
        return Err("the status sprite did not decode".into());
    };
    button.setImage(Some(&image));
    button.setToolTip(Some(&NSString::from_str("Mojito")));
    item.setMenu(Some(&menu));

    STATUS_ITEM.with_borrow_mut(|slot| {
        *slot = Some(StatusItemController { item, target });
    });
    Ok(())
}

/// Badge the item when self-heal reinstalls the hidutil mapping.
/// NSStatusItem has no badge API; an "!" beside the drink is the honest signal.
pub fn set_needs_attention(flag: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    STATUS_ITEM.with_borrow(|slot| {
        let Some(controller) = slot.as_ref() else {
            return;
        };
        let Some(button) = controller.item.button(mtm) else {
            return;
        };
        let title = if flag { "!" } else { "" };
        button.setTitle(&NSString::from_str(title));
    });
}

/// Activate this process: the settings child spawns without an activation
/// ceremony, and its window must land frontmost and key.
pub fn activate_self() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: main thread, called after the run loop started.
    #[allow(deprecated)] // activateIgnoringOtherApps is the reliable self-activation
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
}

/// True when another process of this bundle is already running; activates
/// it so the user lands somewhere visible. Two live instances would both
/// intercept keys and fight over the overlay.
pub fn activate_existing() -> bool {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    use objc2_foundation::NSString;
    let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(
        &NSString::from_str("com.mojito.app"),
    );
    let own = std::process::id() as i32;
    let other = apps
        .to_vec()
        .into_iter()
        .find(|app| app.processIdentifier() != own && !app.isTerminated());
    match other {
        Some(existing) => {
            let _ = existing.activateWithOptions(NSApplicationActivationOptions::empty());
            true
        }
        None => false,
    }
}

fn add_item(
    menu: &NSMenu,
    target: &MenuTarget,
    title: &str,
    action: objc2::runtime::Sel,
    mtm: MainThreadMarker,
) -> Retained<NSMenuItem> {
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            Some(action),
            &NSString::new(),
        )
    };
    unsafe { item.setTarget(Some(target)) };
    menu.addItem(&item);
    item
}

fn plain_item(title: &str, mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            None,
            &NSString::new(),
        )
    }
}
