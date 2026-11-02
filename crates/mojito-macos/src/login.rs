//! Start at Login via SMAppService. One small toggle; the status item's menu
//! flips it, the settings read-out shows it. Runs on a worker thread per the
//! architecture (register can block on consent UI).

use objc2_service_management::{SMAppService, SMAppServiceStatus};

/// Register or unregister the app, whichever flips it to the other state.
pub fn toggle() {
    std::thread::Builder::new()
        .name("mojito-login".into())
        .spawn(|| {
            if let Err(error) = flip(&main_app_service()) {
                eprintln!("mojito: login item: {error:?}");
            }
        })
        .expect("spawn login toggle");
}

fn main_app_service() -> objc2::rc::Retained<SMAppService> {
    // SAFETY: SMAppService is thread-safe, and the main app service needs
    // no arguments.
    unsafe { SMAppService::mainAppService() }
}

fn flip(service: &SMAppService) -> Result<(), objc2::rc::Retained<objc2_foundation::NSError>> {
    // SAFETY: reading the registration status of a live service.
    let enabled = unsafe { service.status() } == SMAppServiceStatus::Enabled;
    if enabled {
        // SAFETY: unregistering a live service; the error out-parameter is
        // handled by the binding.
        unsafe { service.unregisterAndReturnError() }
    } else {
        // SAFETY: as above, for registration.
        unsafe { service.registerAndReturnError() }
    }
}
