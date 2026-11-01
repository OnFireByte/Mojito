//! Start at Login via SMAppService. One small toggle; the status item's menu
//! flips it, the settings read-out shows it. Runs on a worker thread per the
//! architecture (register can block on consent UI).

use objc2_service_management::{SMAppService, SMAppServiceStatus};

/// Register or unregister the app, whichever flips it to the other state.
pub fn toggle() {
    std::thread::Builder::new()
        .name("mojito-login".into())
        .spawn(|| unsafe {
            let service = SMAppService::mainAppService();
            let result = if service.status() == SMAppServiceStatus::Enabled {
                service.unregisterAndReturnError()
            } else {
                service.registerAndReturnError()
            };
            if let Err(error) = result {
                eprintln!("mojito: login item: {error:?}");
            }
        })
        .expect("spawn login toggle");
}
