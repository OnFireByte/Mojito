//! App icons as small CGImages the switcher can show without keeping the
//! full NSImage representations alive.

use std::collections::HashMap;

use objc2::rc::autoreleasepool;
use objc2_app_kit::{NSImage, NSRunningApplication, NSWorkspace};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext, CGImage,
    CGImageAlphaInfo,
};
use objc2_foundation::NSString;

const ICON_PIXELS: usize = 48;

/// App icons keyed by bundle id, rendered once per app; `None` caches a
/// miss so the row keeps its monogram without retrying.
#[derive(Default)]
pub struct IconCache {
    icons: HashMap<String, Option<CFRetained<CGImage>>>,
}

impl IconCache {
    pub fn get(&mut self, bundle: &str) -> Option<CFRetained<CGImage>> {
        self.icons
            .entry(bundle.to_string())
            .or_insert_with(|| app_icon(bundle, ICON_PIXELS))
            .clone()
    }
}

/// The app's icon rendered into a `size` x `size` pixel image. The running
/// instance's icon comes first; the bundle's file icon covers pinned-but-
/// quit apps. Call on the main thread (AppKit).
fn app_icon(bundle: &str, size: usize) -> Option<CFRetained<CGImage>> {
    autoreleasepool(|_| {
        let bundle = NSString::from_str(bundle);
        let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(&bundle);
        let running = apps.to_vec().into_iter().find(|app| !app.isTerminated());
        let image = match running {
            Some(app) => app.icon()?,
            None => {
                let url = NSWorkspace::sharedWorkspace()
                    .URLForApplicationWithBundleIdentifier(&bundle)?;
                let path = url.path()?;
                NSWorkspace::sharedWorkspace().iconForFile(&path)
            }
        };
        render(&image, size)
    })
}

fn render(image: &NSImage, size: usize) -> Option<CFRetained<CGImage>> {
    // SAFETY: a null proposed rect asks for the image's own size, and no
    // context or hints are passed.
    let source =
        unsafe { image.CGImageForProposedRect_context_hints(std::ptr::null_mut(), None, None) }?;
    let space = CGColorSpace::new_device_rgb()?;
    let context = unsafe {
        CGBitmapContextCreate(
            std::ptr::null_mut(),
            size,
            size,
            8,
            size * 4,
            Some(&space),
            CGImageAlphaInfo::PremultipliedLast.0,
        )
    }?;
    let side = size as f64;
    CGContext::draw_image(
        Some(&context),
        CGRect::new(CGPoint::ZERO, CGSize::new(side, side)),
        Some(&source),
    );
    CGBitmapContextCreateImage(Some(&context))
}
