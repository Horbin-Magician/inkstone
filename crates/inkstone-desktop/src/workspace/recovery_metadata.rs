//! Shared presentation metadata; never used as restore authorization.
use std::time::SystemTime;

#[derive(Clone, Copy)]
pub(super) enum Source {
    Draft,
    History,
    Trash,
    SyncBackup,
}

pub(super) struct Metadata {
    pub source: Source,
    pub modified: Option<SystemTime>,
    pub bytes: Option<u64>,
}

impl Metadata {
    pub fn label(&self) -> String {
        let (source, size_kind) = match self.source {
            Source::Draft => ("未保存草稿", "记录文件"),
            Source::History => ("保存历史", "记录文件"),
            Source::Trash => ("回收站", "恢复内容"),
            Source::SyncBackup => ("同步备份", "恢复内容"),
        };
        let time = self
            .modified
            .map(|time| {
                let time: chrono::DateTime<chrono::Local> = time.into();
                time.format("%Y-%m-%d %H:%M:%S").to_string()
            })
            .unwrap_or_else(|| "时间未知".into());
        let size = self
            .bytes
            .map(|bytes| format!("{bytes} 字节（{:.1} KiB）", bytes as f64 / 1024.))
            .unwrap_or_else(|| "大小未知".into());
        format!("{source} · {time} · {size_kind} {size}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_semantics_and_unknown_values_remain_distinct() {
        for (source, kind) in [
            (Source::Draft, "记录文件"),
            (Source::History, "记录文件"),
            (Source::Trash, "恢复内容"),
            (Source::SyncBackup, "恢复内容"),
        ] {
            let mut metadata = Metadata {
                source,
                modified: None,
                bytes: None,
            };
            let unknown = metadata.label();
            assert!(unknown.contains("时间未知") && unknown.contains("大小未知"));
            assert!(unknown.contains(kind));
            metadata.bytes = Some(0);
            assert!(metadata.label().contains("0 字节（0.0 KiB）"));
            metadata.bytes = Some(1025);
            assert!(metadata.label().contains("1025 字节"));
        }
    }
}
