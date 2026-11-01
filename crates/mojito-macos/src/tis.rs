//! Input-source cycling via TIS. Main thread only; the list is the enabled,
//! select-capable keyboard input sources in system order.

/// The enabled input sources, ordered as the system orders them.
pub fn enabled() -> Vec<InputSource> {
    todo!("tis::enabled")
}

pub struct InputSource {
    pub id: String,
    pub name: String,
}

/// Select the next source after the current one; wraps around.
pub fn cycle() -> Result<(), String> {
    todo!("tis::cycle")
}
