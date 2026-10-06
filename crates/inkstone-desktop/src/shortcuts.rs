//! Platform translation applies only to defaults, never persisted user bindings.
pub(crate) fn command_default(key: &str) -> String {
    if cfg!(target_os = "macos") {
        match key {
            "ctrl-tab" | "ctrl-shift-tab" => return key.into(),
            "ctrl-h" => return "cmd-alt-f".into(),
            _ => {}
        }
        if let Some(rest) = key.strip_prefix("ctrl-") {
            return format!("cmd-{rest}");
        }
    }
    key.into()
}
