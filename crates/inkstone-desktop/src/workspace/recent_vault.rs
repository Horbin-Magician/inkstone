//! Persist only accepted vault selections, in acceptance order across windows.
use std::{
    path::{Path, PathBuf},
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Default)]
struct Writer {
    latest: AtomicU64,
    writing: Mutex<()>,
}
impl Writer {
    fn reserve(&self) -> u64 {
        self.latest.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
    }

    fn write(&self, ticket: u64, destination: &Path, root: &Path) -> std::io::Result<()> {
        let _guard = self
            .writing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.latest.load(Ordering::Acquire) != ticket {
            return Ok(());
        }
        std::fs::write(destination, root.to_string_lossy().as_bytes())
    }
}

pub(super) fn remember(root: PathBuf, cx: &gpui::App) {
    static WRITER: LazyLock<Writer> = LazyLock::new(Writer::default);
    let ticket = WRITER.reserve();
    let destination = super::app_dir().join("recent.txt");
    cx.background_executor()
        .spawn(async move {
            let _ = WRITER.write(ticket, &destination, &root);
        })
        .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_writes_cannot_replace_a_newer_accepted_vault() {
        let directory =
            std::env::temp_dir().join(format!("inkstone-recent-writer-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let destination = directory.join("recent.txt");
        let writer = Writer::default();
        let old = writer.reserve();
        let new = writer.reserve();
        writer
            .write(new, &destination, Path::new("新笔记库 😀"))
            .unwrap();
        writer.write(old, &destination, Path::new("old")).unwrap();
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "新笔记库 😀"
        );
        let latest = writer.reserve();
        writer
            .write(new, &destination, Path::new("outdated"))
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "新笔记库 😀"
        );
        writer
            .write(latest, &destination, Path::new("latest"))
            .unwrap();
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), "latest");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
