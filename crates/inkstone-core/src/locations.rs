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
        index
            .notes
            .keys()
            .map(|to| {
                // Distinct directory spellings can coexist on case-sensitive volumes.
                // Shortening across them may resolve to a different local note.
                let local = to.parent() == from.parent();
                self.note_target(from, to, local || index.has_unique_stem(to))
            })
            .collect()
    }
    fn note_target(self, from: &Path, to: &Path, short_resolves: bool) -> String {
        let stem = to.with_extension("");
        let absolute = stem
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        if self == Self::Shortest {
            let name = stem.file_name().unwrap_or_default().to_string_lossy();
            if short_resolves {
                return name.into_owned();
            }
        }
        if self == Self::Relative {
            let relative = relative_path(from, &stem)
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
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
                *folder = new
                    .join(suffix)
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/");
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
        let folder = PathBuf::from(folder.trim().replace(std::path::MAIN_SEPARATOR, "/"));
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
    let unique = format == LinkFormat::Shortest
        && from.parent() != to.parent()
        && files
            .iter()
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&name))
            })
            .take(2)
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
        .add(b']')
        .add(b'\\')
        .add(b'<')
        .add(b'>');
    let label = name
        .replace(['\r', '\n'], " ")
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('<', "\\<")
        .replace('>', "\\>");
    format!(
        "{prefix}[{label}]({})",
        percent_encoding::utf8_percent_encode(&target, ESCAPE)
    )
}

/// Whether a literal target path needs Markdown encoding instead of Wiki syntax.
pub fn requires_markdown_link(target: &str) -> bool {
    target.contains(['#', '\\']) || requires_markdown_text(target)
}

/// Wiki labels and fragments must not introduce markup or split the link.
pub fn requires_markdown_text(text: &str) -> bool {
    text.contains(['|', '[', ']', '<', '>', '\r', '\n'])
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
            let target = relative_path(from, to)
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            if target.contains('/') && !target.starts_with("../") {
                format!("./{target}")
            } else {
                target
            }
        }
        _ => format!(
            "/{}",
            to.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/")
        ),
    };
    let markdown = markdown || requires_markdown_link(&target);
    if !markdown && target.starts_with('/') && target[1..].contains('/') {
        target.remove(0);
    }
    (target, markdown)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn relocated_placement_preserves_literal_backslashes_on_disk() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        std::fs::create_dir(&root).unwrap();
        let vault = crate::vault::Vault::open(root.clone(), temp.path().join("recovery")).unwrap();
        let mut settings = Locations {
            notes: Location::Folder,
            note_folder: "old/notes".into(),
            attachments: Location::Folder,
            attachment_folder: "old/assets".into(),
        };
        settings.relocate(Path::new("old"), Path::new(r"new\folder"));
        assert_eq!(
            settings.directory(None, false).unwrap(),
            Path::new(r"new\folder/notes")
        );
        let folder = settings.directory(None, true).unwrap();
        let stored = vault
            .store_attachment_to(&folder, "图.png", b"attachment")
            .unwrap();
        assert_eq!(stored, Path::new(r"new\folder/assets/图.png"));
        assert_eq!(std::fs::read(root.join(&stored)).unwrap(), b"attachment");
        assert!(!root.join("new/folder").exists());
        settings.relocate(Path::new(r"new\folder"), Path::new("final"));
        assert_eq!(
            settings.directory(None, true).unwrap(),
            Path::new("final/assets")
        );
    }

    #[test]
    fn attachment_links_preserve_syntax_sensitive_filenames() {
        for name in [
            "line\nbreak.png",
            "paragraph\n\nbreak.png",
            "carriage\rreturn.png",
            "<image>.png",
            "less<than.png",
            "greater>than.png",
        ]
        .into_iter()
        .chain(cfg!(unix).then_some(r"literal\name.png"))
        .chain(cfg!(unix).then_some(r"literal\[name].png"))
        {
            let target = PathBuf::from(format!("assets/{name}"));
            let files = vec![target.clone()];
            for format in [
                LinkFormat::Shortest,
                LinkFormat::Relative,
                LinkFormat::Absolute,
            ] {
                for markdown in [false, true] {
                    for embed in [false, true] {
                        let text = attachment_link(
                            Path::new("source.md"),
                            &target,
                            &files,
                            format,
                            markdown,
                            embed,
                        );
                        let parsed = crate::index::parse(&text);
                        assert_eq!(parsed.destinations.len(), 1, "{text:?}");
                        let decoded =
                            percent_encoding::percent_decode_str(&parsed.destinations[0].1)
                                .decode_utf8()
                                .unwrap();
                        assert!(decoded.ends_with(name), "{text:?}");
                        assert!(!text.contains(['\r', '\n']), "{text:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn note_link_formats_resolve_duplicates_and_nested_paths() {
        use crate::index::{Index, Resolution};
        let mut index = Index::default();
        for name in [
            "a/same.md",
            "A/same.md",
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
