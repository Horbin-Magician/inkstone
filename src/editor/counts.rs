//! Cached selection and document word counts.

use super::*;

impl EditorPane {
    pub fn text_counts(&mut self, cx: &mut Context<Self>) -> inkstone::word_count::Counts {
        let editor = self.editor.read(cx);
        let source = editor.value();
        let selection = if self.reading || editor.selected_range().is_empty() {
            0..0
        } else {
            editor.selected_range()
        };
        if let Some(cache) = &self.count_cache
            && cache.source == source
            && cache.selection == selection
        {
            return cache.counts;
        }
        self.count_revision = self.count_revision.wrapping_add(1);
        let revision = self.count_revision;
        self.count_task = None;
        let previous = self
            .count_cache
            .as_ref()
            .map_or(Default::default(), |cache| cache.counts);
        let length = if selection.is_empty() {
            source.len()
        } else {
            selection.len()
        };
        self.count_cache = Some(CountSnapshot {
            source: source.clone(),
            selection: selection.clone(),
            counts: previous,
        });
        let calculate = move || {
            let text = if selection.is_empty() {
                inkstone::word_count::document_body(&source)
            } else {
                &source[selection]
            };
            inkstone::word_count::count(text)
        };
        if length <= 4096 {
            let counts = calculate();
            self.count_cache.as_mut().unwrap().counts = counts;
            return counts;
        }
        let timer = cx
            .background_executor()
            .timer(std::time::Duration::from_millis(200));
        self.count_task = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let counts = cx
                .background_executor()
                .spawn(async move { calculate() })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.count_revision == revision {
                    if let Some(cache) = &mut this.count_cache {
                        cache.counts = counts;
                    }
                    cx.emit(EditorEvent::CountsChanged);
                    cx.notify();
                }
            });
        }));
        previous
    }
}
