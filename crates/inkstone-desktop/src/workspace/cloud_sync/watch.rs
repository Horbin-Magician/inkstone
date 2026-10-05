//! Coalesced, bounded watcher classification off the UI thread.
use super::*;
use std::{collections::BTreeSet, time::Instant};

#[derive(Default)]
pub(super) struct Watch {
    paths: BTreeSet<PathBuf>,
    first: Option<Instant>,
    due: Option<Instant>,
    checking: bool,
    revision: u64,
    pub epoch: u64,
}
impl Watch {
    pub fn reset(&mut self) {
        let revision = self.revision.wrapping_add(1);
        *self = Self {
            revision,
            ..Self::default()
        };
    }
    fn record(&mut self, paths: impl IntoIterator<Item = PathBuf>, now: Instant) {
        let mut received = false;
        for path in paths {
            if path
                .components()
                .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
            {
                continue;
            }
            received = true;
            if self.paths.contains(&PathBuf::new()) {
                break;
            }
            if path.as_os_str().is_empty() || self.paths.len() >= 1024 {
                self.paths.clear();
                self.paths.insert(PathBuf::new());
                break;
            }
            self.paths.insert(path);
        }
        if received && !self.paths.is_empty() {
            let first = *self.first.get_or_insert(now);
            self.due = Some((now + Duration::from_secs(3)).min(first + Duration::from_secs(5)));
        }
    }
}
impl Workspace {
    pub(in crate::workspace) fn note_sync_watch_paths(&mut self, paths: Vec<PathBuf>) {
        if self.ui.prefs.webdav.auto && !self.ui.prefs.webdav.url.trim().is_empty() {
            self.ui.cloud_sync.watch.record(paths, Instant::now());
        }
    }
    pub(in crate::workspace) fn tick_sync_watch(&mut self, cx: &mut Context<Self>) {
        self.tick_sync_watch_at(Instant::now(), cx);
    }
    pub(super) fn tick_sync_watch_at(&mut self, now: Instant, cx: &mut Context<Self>) {
        if !self.ui.prefs.webdav.auto {
            self.ui.cloud_sync.watch.reset();
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let state = &mut self.ui.cloud_sync;
        if self.loading
            || self.ui.file_operation
            || state.busy
            || state.pending
            || state.again
            || state.watch.checking
            || state.watch.due.is_none_or(|due| now < due)
        {
            return;
        }
        let settings = self.ui.prefs.webdav.clone();
        let password = state.password.read(cx).value().to_string();
        let watch = &mut state.watch;
        let paths = std::mem::take(&mut watch.paths);
        watch.first = None;
        watch.due = None;
        watch.checking = true;
        let revision = watch.revision;
        let epoch = watch.epoch;
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            let result = WebDav::new(&settings, &password)
                .and_then(|remote| sync::changed_since_sync(&vault, &remote.identity(), &paths));
            (result, paths, settings)
        });
        cx.spawn(async move |this, cx| {
            let (result, paths, settings) = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.ui.cloud_sync.watch.revision != revision {
                    return;
                }
                let state = &mut this.ui.cloud_sync;
                state.watch.checking = false;
                if !this.ui.prefs.webdav.auto
                    || this.ui.prefs.webdav.url != settings.url
                    || this.ui.prefs.webdav.username != settings.username
                {
                    return;
                }
                if state.watch.epoch != epoch {
                    // A sync started during classification; recheck against its
                    // completed baseline rather than uploading its own downloads.
                    state.watch.record(paths, Instant::now());
                    return;
                }
                match result {
                    Ok(false) => (),
                    Ok(true) => this.schedule_auto_sync(false),
                    Err(error) => {
                        this.cloud_message(
                            format!("外部变更检查失败，将通过同步重试：{error}"),
                            cx,
                        );
                        this.schedule_auto_sync(false);
                        this.ui.cloud_sync.defer_retry = true;
                    }
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[test]
    fn events_coalesce_bound_memory_and_do_not_starve_under_continuous_changes() {
        let mut watch = Watch::default();
        let now = Instant::now();
        for index in 0..10 {
            watch.record([PathBuf::from("note.md")], now + Duration::from_secs(index));
        }
        assert_eq!(watch.paths.len(), 1);
        assert_eq!(watch.due, Some(now + Duration::from_secs(5)));
        watch.record([], now + Duration::from_secs(20));
        assert_eq!(watch.due, Some(now + Duration::from_secs(5)));
        watch.record((0..2000).map(|i| PathBuf::from(format!("{i}.bin"))), now);
        assert_eq!(watch.paths, BTreeSet::from([PathBuf::new()]));
        let revision = watch.revision;
        watch.reset();
        assert_ne!(watch.revision, revision);
        watch.record([PathBuf::from(".inkstone-workspace.json")], now);
        assert!(watch.due.is_none());
    }
}
