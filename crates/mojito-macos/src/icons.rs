//! App icons as straight RGBA the UI can hand to rune's `Pixels`.

use objc2_app_kit::{NSImage, NSRunningApplication, NSWorkspace};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo};
use objc2_foundation::NSString;

/// The app's icon rendered into `size` x `size` RGBA bytes. The running
/// instance's icon comes first; the bundle's file icon covers pinned-but-
/// quit apps. Call on the main thread (AppKit).
pub fn app_icon_rgba(bundle: &str, size: u32) -> Option<Vec<u8>> {
    let bundle = NSString::from_str(bundle);
    let image = {
        let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(&bundle);
        let running = apps.to_vec().into_iter().find(|app| !app.isTerminated());
        match running {
            Some(app) => app.icon()?,
            None => {
                let url = NSWorkspace::sharedWorkspace()
                    .URLForApplicationWithBundleIdentifier(&bundle)?;
                let path = url.path()?;
                NSWorkspace::sharedWorkspace().iconForFile(&path)
            }
        }
    };
    render_rgba(&image, size)
}

/// Draw an NSImage into a square RGBA bitmap context and copy the bytes.
fn render_rgba(image: &NSImage, size: u32) -> Option<Vec<u8>> {
    // SAFETY: a null proposed rect asks for the image's own size, and no
    // context or hints are passed.
    let cg_image =
        unsafe { image.CGImageForProposedRect_context_hints(std::ptr::null_mut(), None, None) }?;
    let space = CGColorSpace::new_device_rgb()?;
    let side = size as usize;
    let stride = side * 4;
    let mut buffer = vec![0_u8; stride * side];
    // SAFETY: `buffer` holds exactly `stride * side` bytes and outlives the
    // context, which is dropped at the end of this function before `buffer`
    // is returned.
    let context = unsafe {
        CGBitmapContextCreate(
            buffer.as_mut_ptr().cast(),
            side,
            side,
            8,
            stride,
            Some(&space),
            CGImageAlphaInfo::PremultipliedLast.0,
        )
    }?;
    // CG images draw upright into a bitmap context without a flip; the
    // earlier translate+scale flip turned them upside down.
    let bounds = CGRect::new(CGPoint::ZERO, CGSize::new(f64::from(size), f64::from(size)));
    CGContext::draw_image(Some(&context), bounds, Some(&cg_image));
    drop(context);
    Some(buffer)
}
