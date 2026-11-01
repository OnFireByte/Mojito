//! App icons as straight RGBA the UI can hand to rune's `Pixels`.

use std::ffi::c_void;
use std::ptr;

use objc2::msg_send;
use objc2_app_kit::{NSRunningApplication, NSWorkspace};
use objc2_foundation::{NSString, NSURL};

type ImageRef = *const c_void;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> ImageRef;
    fn CGColorSpaceRelease(space: ImageRef);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: ImageRef,
        bitmap_info: u32,
    ) -> *mut c_void;
    fn CGContextDrawImage(context: *mut c_void, rect: CGRect, image: ImageRef);
    fn CGContextTranslateCTM(context: *mut c_void, x: f64, y: f64);
    fn CGContextScaleCTM(context: *mut c_void, sx: f64, sy: f64);
    fn CGBitmapContextCreateImageData(context: *mut c_void) -> ImageRef;
    fn CFDataGetBytePtr(data: ImageRef) -> *const u8;
    fn CFDataGetLength(data: ImageRef) -> isize;
    fn CFRelease(value: ImageRef);
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// `kCGImageAlphaPremultipliedLast`.
const PREMULTIPLIED_LAST: u32 = 1;

/// The app's icon rendered into `size` x `size` RGBA bytes. The running
/// instance's icon comes first; the bundle's file icon covers pinned-but-
/// quit apps. Call on the main thread (AppKit).
pub fn app_icon_rgba(bundle: &str, size: u32) -> Option<Vec<u8>> {
    let bundle = NSString::from_str(bundle);
    // SAFETY: AppKit calls on the main thread; returned NSImages are
    // retained and released by their wrappers.
    let image = unsafe {
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
fn render_rgba(image: &objc2_app_kit::NSImage, size: u32) -> Option<Vec<u8>> {
    // SAFETY: `image` is a live NSImage; every CG object created here is
    // released on all paths, and the buffer outlives the context.
    unsafe {
        let cg: ImageRef = msg_send![
            image,
            CGImageForProposedRect: ptr::null_mut::<c_void>(),
            context: ptr::null_mut::<c_void>(),
            hints: ptr::null_mut::<c_void>(),
        ];
        if cg.is_null() {
            return None;
        }
        let space = CGColorSpaceCreateDeviceRGB();
        if space.is_null() {
            return None;
        }
        let stride = size as usize * 4;
        let mut buffer = vec![0_u8; stride * size as usize];
        let context = CGBitmapContextCreate(
            buffer.as_mut_ptr().cast(),
            size as usize,
            size as usize,
            8,
            stride,
            space,
            PREMULTIPLIED_LAST,
        );
        if context.is_null() {
            CGColorSpaceRelease(space);
            return None;
        }
        // CG bitmaps have a bottom-up origin; flip so the icon is upright.
        CGContextTranslateCTM(context, 0.0, f64::from(size));
        CGContextScaleCTM(context, 1.0, -1.0);
        CGContextDrawImage(
            context,
            CGRect { x: 0.0, y: 0.0, width: f64::from(size), height: f64::from(size) },
            cg,
        );
        let data = CGBitmapContextCreateImageData(context);
        let bytes = if data.is_null() {
            None
        } else {
            let length = CFDataGetLength(data);
            if length <= 0 {
                None
            } else {
                Some(std::slice::from_raw_parts(CFDataGetBytePtr(data), length as usize).to_vec())
            }
        };
        CFRelease(data);
        CFRelease(context);
        CGColorSpaceRelease(space);
        bytes
    }
}
