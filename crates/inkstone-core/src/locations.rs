//! Vault-relative placement of newly created notes and attachments.
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkFormat {
    #[default]
    Shortest,
    Relative,
    Absolute,
}

impl LinkFormat {
    pub fn note_targets(self, index: &crate::index::Index, from: &Path) -> Vec<String> {
        let mut names = std::collections::HashMap::<String, usize>::new();
        for path in index.notes.keys() {
            *names
                .entry(
                    path.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase(),
                )
                .or_default() += 1;
        }
        index
            .notes
            .keys()
            .map(|to| {
                let name = to
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                let local = to
                    .parent()
                    .unwrap_or(Path::new(""))
                    .to_string_lossy()
                    .eq_ignore_ascii_case(
                        &from.parent().unwrap_or(Path::new("")).to_string_lossy(),
                    );
                self.note_target(from, to, local || names.get(&name) == Some(&1))
            })
            .collect()
    }
    fn note_target(self, from: &Path, to: &Path, short_resolves: bool) -> String {
        let stem = to.with_extension("");
        let absolute = stem.to_string_lossy().replace('\\', "/");
        if self == Self::Shortest {
            let name = stem.file_name().unwrap_or_default().to_string_lossy();
            if short_resolves {
                return name.into_owned();
            }
        }
        if self == Self::Relative {
            let relative = relative_path(from, &stem)
                .to_string_lossy()
                .replace('\\', "/");
            return if relative.starts_with("../") || !relative.contains('/') {
                relative
            } else {
                format!("./{relative}")
            };
        }
        if absolute.contains('/') {
            absolute
        } else {
            format!("/{absolute}")
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Location {
    #[default]
    Root,
    Current,
    Folder,
    Subfolder,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Locations {
    pub notes: Location,
    pub note_folder: String,
    pub attachments: Location,
    pub attachment_folder: String,
}
impl Default for Locations {
    fn default() -> Self {
        Self {
            notes: Location::Root,
            note_folder: String::new(),
            attachments: Location::Folder,
            attachment_folder: "附件".into(),
        }
    }
}
impl Locations {
    pub fn relocate(&mut self, old: &Path, new: &Path) {
        for (mode, folder) in [
            (self.notes, &mut self.note_folder),
            (self.attachments, &mut self.attachment_folder),
        ] {
            if mode == Location::Folder
                && let Ok(suffix) = Path::new(folder).strip_prefix(old)
            {
                *folder = new.join(suffix).to_string_lossy().replace('\\', "/");
            }
        }
    }
    pub fn directory(
        &self,
        current: Option<&Path>,
        attachment: bool,
    ) -> Result<PathBuf, &'static str> {
        let (mode, folder) = if attachment {
            (self.attachments, &self.attachment_folder)
        } else {
            (self.notes, &self.note_folder)
        };
        let current = current.and_then(Path::parent).unwrap_or(Path::new(""));
        if mode == Location::Root {
            return Ok(PathBuf::new());
        }
        if mode == Location::Current {
            return Ok(current.to_path_buf());
        }
        let folder = PathBuf::from(folder.trim().replace('\\', "/"));
        if folder.as_os_str().is_empty()
            || folder.components().any(|c| {
                !matches!(c, Component::Normal(_))
                    || c.as_os_str().to_string_lossy().starts_with('.')
                    || c.as_os_str().to_string_lossy().contains(':')
            })
        {
            return Err("请填写库内文件夹，不能使用绝对路径、隐藏目录或 ..。");
        }
        Ok(if mode == Location::Subfolder {
            current.join(folder)
        } else {
            folder
        })
    }
}
pub fn relative_path(from: &Path, to: &Path) -> PathBuf {
    let a: Vec<_> = from
        .parent()
        .unwrap_or(Path::new(""))
        .components()
        .collect();
    let b: Vec<_> = to.components().collect();
    let common = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
    let mut result = PathBuf::new();
    for _ in common..a.len() {
        result.push("..");
    }
    for c in &b[common..] {
        result.push(c.as_os_str());
    }
    result
}

pub fn attachment_link(
    from: &Path,
    to: &Path,
    files: &[PathBuf],
    format: LinkFormat,
    markdown: bool,
    embed: bool,
) -> String {
    let name = to.file_name().unwrap_or_default().to_string_lossy();
    let unique = files
        .iter()
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&name))
        })
        .count()
        == 1;
    let (target, markdown) = attachment_destination(from, to, format, markdown, unique);
    let prefix = if embed { "!" } else { "" };
    if !markdown {
        return format!("{prefix}[[{target}]]");
    }
    const ESCAPE: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
        .add(b' ')
        .add(b'#')
        .add(b'%')
        .add(b'(')
        .add(b')')
        .add(b'[')
        .add(b']');
    let label = name
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]");
    format!(
        "{prefix}[{label}]({})",
        percent_encoding::utf8_percent_encode(&target, ESCAPE)
    )
}

/// Return the unescaped destination and effective syntax for an attachment.
pub fn attachment_destination(
    from: &Path,
    to: &Path,
    format: LinkFormat,
    markdown: bool,
    unique: bool,
) -> (String, bool) {
    let name = to.file_name().unwrap_or_default().to_string_lossy();
    let local = to.parent() == from.parent();
    let mut target = match format {
        LinkFormat::Shortest if unique || local => name.to_string(),
        LinkFormat::Relative => {
            let target = relative_path(from, to).to_string_lossy().replace('\\', "/");
            if target.contains('/') && !target.starts_with("../") {
                format!("./{target}")
            } else {
                target
            }
        }
        _ => format!("/{}", to.to_string_lossy().replace('\\', "/")),
    };
    let markdown = markdown || target.contains(['#', '|', '[', ']']);
    if !markdown && target.starts_with('/') && target[1..].contains('/') {
        target.remove(0);
    }
    (target, markdown)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn note_link_formats_resolve_duplicates_and_nested_paths() {
        use crate::index::{Index, Resolution};
        let mut index = Index::default();
        for name in [
            "a/same.md",
            "same.md",
            "a/sub/中文.2026.md",
            "other/unique.md",
        ] {
            index.update(name.into(), String::new());
        }
        let from = Path::new("a/source.md");
        for format in [
            LinkFormat::Shortest,
            LinkFormat::Relative,
            LinkFormat::Absolute,
        ] {
            let targets = format.note_targets(&index, from);
            for (path, target) in index.notes.keys().zip(&targets) {
                assert_eq!(
                    index.resolve(from, target),
                    Resolution::Found(path.clone()),
                    "{format:?}: {target}"
                );
            }
            match format {
                LinkFormat::Shortest => {
                    assert!(targets.contains(&"same".into()));
                    assert!(targets.contains(&"/same".into()));
                    assert!(targets.contains(&"unique".into()));
                }
                LinkFormat::Relative => {
                    assert!(targets.contains(&"./sub/中文.2026".into()));
                    assert!(targets.contains(&"../same".into()));
                }
                LinkFormat::Absolute => assert!(targets.contains(&"a/sub/中文.2026".into())),
            }
        }
    }
    #[test]
    fn placement_modes_and_relative_links_stay_inside_vault() {
        let mut settings = Locations::default();
        let current = Some(Path::new("项目/笔记.md"));
        assert_eq!(settings.directory(current, false).unwrap(), Path::new(""));
        settings.notes = Location::Current;
        assert_eq!(
            settings.directory(current, false).unwrap(),
            Path::new("项目")
        );
        settings.attachments = Location::Subfolder;
        settings.attachment_folder = "图片/原件".into();
        assert_eq!(
            settings.directory(current, true).unwrap(),
            Path::new("项目/图片/原件")
        );
        for bad in ["../out", "C:/out", "/out", ".inkstone-trash", ""] {
            settings.attachment_folder = bad.into();
            assert!(settings.directory(current, true).is_err());
        }
        assert_eq!(
            relative_path(Path::new("项目/a.md"), Path::new("项目/图.png")),
            Path::new("图.png")
        );
        assert_eq!(
            relative_path(Path::new("项目/a.md"), Path::new("图.png")),
            Path::new("../图.png")
        );
        settings.notes = Location::Folder;
        settings.note_folder = "old/notes".into();
        settings.attachments = Location::Subfolder;
        settings.attachment_folder = "old".into();
        settings.relocate(Path::new("old"), Path::new("new"));
        assert_eq!(settings.note_folder, "new/notes");
        assert_eq!(settings.attachment_folder, "old");
    }
}
