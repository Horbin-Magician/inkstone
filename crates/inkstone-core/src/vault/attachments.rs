//! Attachment inventory and recoverable cleanup of suspected unused files.
use super::*;
use crate::{
    index::Index,
    rendering::{self, Reference},
};
use markdown_parser::mdast::Node;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Pdf,
    Media,
    Other,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Image => "图片",
            Self::Pdf => "PDF",
            Self::Media => "音视频",
            Self::Other => "其他",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Attachment {
    pub path: PathBuf,
    pub bytes: u64,
    pub modified: SystemTime,
    pub kind: Kind,
    pub references: Vec<PathBuf>,
}
#[derive(Default)]
pub struct Inventory {
    pub entries: Vec<Attachment>,
    pub unreadable: usize,
}
fn kind(path: &Path) -> Kind {
    match path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
        .as_str()
    {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif" => Kind::Image,
        "pdf" => Kind::Pdf,
        "mp3" | "wav" | "ogg" | "m4a" | "mp4" | "webm" | "mov" | "flac" => Kind::Media,
        _ => Kind::Other,
    }
}
pub fn scan(vault: &Vault, drafts: &BTreeMap<PathBuf, String>) -> Result<Inventory, VaultError> {
    let mut index = Index::build(vault)?;
    for (path, text) in drafts {
        Vault::validate_relative(path)?;
        index.update(path.clone(), text.clone());
    }
    let mut uses: BTreeMap<PathBuf, BTreeSet<PathBuf>> = BTreeMap::new();
    for (path, note) in &index.notes {
        let snapshot = crate::syntax::Snapshot::new(&note.text);
        let mut refs: Vec<_> = note
            .parsed
            .links
            .iter()
            .map(|l| (l.target.clone(), true))
            .collect();
        // Read decoded AST destinations, including reference definitions and escapes.
        fn walk(node: &Node, refs: &mut Vec<(String, bool)>) {
            match node {
                Node::Link(n) => refs.push((n.url.clone(), false)),
                Node::Image(n) => refs.push((n.url.clone(), false)),
                Node::Definition(n) => refs.push((n.url.clone(), false)),
                _ => (),
            }
            if let Some(children) = node.children() {
                for n in children {
                    walk(n, refs);
                }
            }
        }
        if let Some(ast) = &snapshot.ast {
            walk(ast, &mut refs);
            refs.extend(
                crate::syntax::html_destinations(ast, &snapshot.structural)
                    .into_iter()
                    .map(|(_, url)| (url, false)),
            );
        }
        for (target, wiki) in refs {
            if let Some(full) = rendering::asset_path(
                &vault.root,
                &Reference {
                    from: path.clone(),
                    target,
                    wiki,
                },
                &index.files,
            ) && let Ok(relative) = full.strip_prefix(&vault.root)
            {
                uses.entry(relative.into())
                    .or_default()
                    .insert(path.clone());
            }
        }
    }
    let mut entries = vec![];
    for path in &index.files {
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        {
            continue;
        }
        let full = vault.regular_file_path(path)?;
        let meta = fs::symlink_metadata(full)?;
        if !meta.is_file() || is_reparse(&meta) {
            continue;
        }
        entries.push(Attachment {
            path: path.clone(),
            bytes: meta.len(),
            modified: meta.modified()?,
            kind: kind(path),
            references: uses.remove(path).unwrap_or_default().into_iter().collect(),
        });
    }
    entries.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    Ok(Inventory {
        entries,
        unreadable: index.errors.len(),
    })
}
/// Re-scan before moving; the whole captured file remains recoverable in owned trash.
pub fn trash_unused(
    vault: &Vault,
    expected: &Attachment,
    drafts: &BTreeMap<PathBuf, String>,
) -> Result<PathBuf, VaultError> {
    let current = scan(vault, drafts)?;
    if current.unreadable > 0 {
        return Err(io::Error::other("存在无法读取的笔记，不能可靠判断附件引用").into());
    }
    let current = current
        .entries
        .iter()
        .find(|e| e.path == expected.path)
        .ok_or_else(|| io::Error::other("附件已移动或消失，请刷新"))?;
    if !current.references.is_empty() {
        return Err(io::Error::other("附件已被笔记引用，已取消清理").into());
    }
    if current.bytes != expected.bytes || current.modified != expected.modified {
        return Err(io::Error::other("附件在预览后已变化，请刷新").into());
    }
    let source = vault.regular_file_path(&current.path)?;
    let meta = fs::symlink_metadata(&source)?;
    if !meta.is_file()
        || is_reparse(&meta)
        || meta.len() != expected.bytes
        || meta.modified()? != expected.modified
    {
        return Err(io::Error::other("附件在清理前已变化").into());
    }
    let entry = vault.create_trash_entry(&current.path)?;
    write_new_synced(&entry.join("payload-name.json"), b"\"content\"")?;
    let dest = entry.join("content");
    move_no_replace(&source, &dest)?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attachment_names_cannot_collide_with_trash_metadata() {
        let root = std::env::temp_dir().join(format!("inkstone-attachments-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        for name in ["original-path.json", "payload-name.json", "content"] {
            fs::write(vault.root.join(name), b"original attachment").unwrap();
            let entry = scan(&vault, &BTreeMap::new())
                .unwrap()
                .entries
                .into_iter()
                .find(|e| e.path == Path::new(name))
                .unwrap();
            trash_unused(&vault, &entry, &BTreeMap::new()).unwrap();
            let trashed = vault
                .trash_entries()
                .unwrap()
                .into_iter()
                .find(|e| e.original == Path::new(name))
                .unwrap();
            vault.restore_trash(&trashed).unwrap();
            assert_eq!(
                fs::read(vault.root.join(name)).unwrap(),
                b"original attachment"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn inventory_counts_markdown_wiki_html_and_unsaved_references() {
        let root = std::env::temp_dir().join(format!("inkstone-attachments-{}", unique_id()));
        fs::create_dir_all(root.join("vault/附件")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        for p in ["图片 a.png", "参考.pdf", "unused.bin"] {
            fs::write(vault.root.join("附件").join(p), [0, 255, 1]).unwrap();
        }
        fs::write(
            vault.root.join("a.md"),
            "![[附件/图片 a.png]]\n\n[资料][pdf]\n\n[pdf]: 附件/参考.pdf\n",
        )
        .unwrap();
        fs::write(vault.root.join("b.md"), "<img src=\"附件/图片%20a.png\">\n").unwrap();
        let inventory = scan(&vault, &BTreeMap::new()).unwrap();
        assert_eq!(inventory.entries.len(), 3);
        let image = inventory
            .entries
            .iter()
            .find(|e| e.kind == Kind::Image)
            .unwrap();
        assert_eq!(image.references.len(), 2);
        assert!(trash_unused(&vault, image, &BTreeMap::new()).is_err());
        let unused = inventory
            .entries
            .iter()
            .find(|e| e.references.is_empty())
            .unwrap();
        let drafts = BTreeMap::from([("a.md".into(), "[新增](附件/unused.bin)".into())]);
        assert!(trash_unused(&vault, unused, &drafts).is_err());
        let original = fs::read(vault.root.join(&unused.path)).unwrap();
        trash_unused(&vault, unused, &BTreeMap::new()).unwrap();
        assert!(!vault.root.join(&unused.path).exists());
        let entry = vault.trash_entries().unwrap().pop().unwrap();
        vault.restore_trash(&entry).unwrap();
        assert_eq!(fs::read(vault.root.join(&unused.path)).unwrap(), original);
        fs::write(vault.root.join(&unused.path), "changed length").unwrap();
        assert!(trash_unused(&vault, unused, &BTreeMap::new()).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unreadable_notes_block_cleanup_and_trash_restore_never_overwrites() {
        let root = std::env::temp_dir().join(format!("inkstone-attachments-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        fs::write(vault.root.join("x.pdf"), "PDF").unwrap();
        let entry = scan(&vault, &BTreeMap::new())
            .unwrap()
            .entries
            .pop()
            .unwrap();
        fs::write(vault.root.join("bad.md"), [255]).unwrap();
        assert!(trash_unused(&vault, &entry, &BTreeMap::new()).is_err());
        fs::write(vault.root.join("bad.md"), "ok").unwrap();
        trash_unused(&vault, &entry, &BTreeMap::new()).unwrap();
        fs::write(vault.root.join("x.pdf"), "new").unwrap();
        assert!(
            vault
                .restore_trash(&vault.trash_entries().unwrap()[0])
                .is_err()
        );
        assert_eq!(fs::read_to_string(vault.root.join("x.pdf")).unwrap(), "new");
        fs::remove_dir_all(root).unwrap();
    }
}
