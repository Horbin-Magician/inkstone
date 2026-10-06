//! Cached selection and document word counts.

use super::*;

impl EditorPane {
    pub fn text_counts(&mut self, cx: &mut Context<Self>) -> inkstone_core::word_count::Counts {
        let editor = self.editor.read(cx);
        let source = editor.text();
        let selection = if self.reading || editor.selected_range().is_empty() {
            0..0
        } else {
            editor.selected_range()
        };
        if let Some(cache) = &self.count_cache
            && cache.source == *source
            && cache.selection == selection
        {
            return cache.counts;
        }
        // Clone the shared rope, not a flattened document. Large count requests
        // materialize text only after the debounce, on the background executor.
        let source = source.clone();
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
            if selection.is_empty() {
                let text = source.to_string();
                inkstone_core::word_count::count(inkstone_core::word_count::document_body(&text))
            } else {
                inkstone_core::word_count::count(&source.slice(selection).to_string())
            }
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

    pub fn properties(&mut self, cx: &App) -> &[inkstone_core::properties::Property] {
        let source = self.editor.read(cx).value();
        if self.property_cache.as_ref().is_none_or(|cache| {
            !std::ptr::eq(cache.source.as_ref(), source.as_ref()) && cache.source != source
        }) {
            self.property_cache = Some(PropertySnapshot {
                properties: inkstone_core::properties::parse(&source),
                source,
            });
        }
        &self.property_cache.as_ref().unwrap().properties
    }
}
