//! Workspace notification text; producers cannot mutate the display buffer directly.
#[derive(Default)]
pub(super) struct Notifications {
    text: String,
}
impl Notifications {
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn publish(&mut self, text: String) {
        self.text = text;
    }
    pub fn append(&mut self, text: &str) {
        self.text.push_str(text);
    }
    /// Clearing a stale search error must not dismiss a newer save/sync notification.
    pub fn clear_matching(&mut self, previous: &str) {
        if !previous.is_empty() && self.text == previous {
            self.text.clear();
        }
    }
}
