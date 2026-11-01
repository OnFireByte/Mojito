//! NSStatusItem via objc2 (hex macos_rune/status_item.rs pattern). Menu:
//! Settings, Start at Login, Restore Caps Lock, About, Quit.

pub enum MenuAction {
    OpenSettings,
    ToggleLogin,
    RestoreCaps,
    Quit,
}

/// Install the status item; actions cross to the main pump over a channel.
pub fn install(tx: std::sync::mpsc::SyncSender<MenuAction>) -> Result<(), String> {
    let _ = tx;
    todo!("status_item::install")
}

/// Badge the item when self-heal reinstalls the hidutil mapping.
pub fn set_needs_attention(bool_flag: bool) {
    let _ = bool_flag;
    todo!("set_needs_attention")
}
