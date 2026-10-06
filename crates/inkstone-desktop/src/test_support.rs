//! Dispatch platform defaults while keeping test cases readable in Windows notation.
//! Custom bindings must remain literal when testing user overrides.
use gpui::VisualTestContext;

pub fn keys(sequence: &str) -> String {
    sequence
        .split_whitespace()
        .map(|key| {
            if cfg!(target_os = "macos") {
                match key {
                    "ctrl-s" => "cmd-s",
                    "ctrl-shift-o" => "cmd-shift-o",
                    "ctrl-o" => "cmd-o",
                    "ctrl-p" => "cmd-p",
                    "ctrl-shift-p" => "cmd-shift-p",
                    "ctrl-shift-f" => "cmd-shift-f",
                    "ctrl-n" => "cmd-n",
                    "ctrl-t" => "cmd-t",
                    "ctrl-w" => "cmd-w",
                    "ctrl-shift-r" => "cmd-shift-r",
                    "ctrl-," => "cmd-,",
                    "ctrl-shift-t" => "cmd-shift-t",
                    "ctrl-e" => "cmd-e",
                    "ctrl-b" => "cmd-b",
                    "ctrl-i" => "cmd-i",
                    "ctrl-k" => "cmd-k",
                    "ctrl-h" => "cmd-alt-f",
                    "ctrl-\\" => "cmd-\\",
                    "ctrl-a" => "cmd-a",
                    "ctrl-c" => "cmd-c",
                    "ctrl-x" => "cmd-x",
                    "ctrl-v" => "cmd-v",
                    "ctrl-z" => "cmd-z",
                    "ctrl-y" => "cmd-shift-z",
                    "ctrl-d" => "cmd-d",
                    "ctrl-shift-l" => "cmd-shift-l",
                    "ctrl-shift-k" => "cmd-shift-k",
                    "ctrl-alt-up" => "cmd-alt-up",
                    "ctrl-alt-down" => "cmd-alt-down",
                    "ctrl-home" => "cmd-up",
                    "ctrl-end" => "cmd-down",
                    "ctrl-shift-home" => "cmd-shift-up",
                    "ctrl-shift-end" => "cmd-shift-down",
                    "alt-l" => "ctrl-l",
                    "ctrl-l" => "cmd-l",
                    "ctrl-1" => "cmd-1",
                    "ctrl-2" => "cmd-2",
                    "ctrl-3" => "cmd-3",
                    "ctrl-4" => "cmd-4",
                    "ctrl-5" => "cmd-5",
                    "ctrl-6" => "cmd-6",
                    "ctrl-7" => "cmd-7",
                    "ctrl-8" => "cmd-8",
                    "ctrl-9" => "cmd-9",
                    _ => key,
                }
            } else if cfg!(target_os = "linux") {
                match key {
                    "ctrl-alt-up" => "shift-alt-up",
                    "ctrl-alt-down" => "shift-alt-down",
                    _ => key,
                }
            } else {
                key
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
pub trait PlatformKeys {
    fn simulate_platform_keystrokes(&mut self, sequence: &str);
}
impl PlatformKeys for VisualTestContext {
    fn simulate_platform_keystrokes(&mut self, sequence: &str) {
        self.simulate_keystrokes(&keys(sequence));
    }
}
