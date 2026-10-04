//! Wiki/Markdown resolution, backlinks, and relocation reference updates.

use super::{Index, Resolution, key};
use std::{
    ops::Range,
    path::{Component, Path, PathBuf},
};

const LINK_PATH_ESCAPE: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
    .add(b' ')
    .add(b'#')
    .add(b'%')
    .add(b'(')
    .add(b')');

fn normalized(path: &Path) -> Option<PathBuf> {
    let mut result = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(p) => result.push(p),
            Component::CurDir => (),
            Component::ParentDir => {
                if !result.pop() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(result)
}

impl Index {
    /// Plan all references against the pre-move index, including outgoing relative links.
    pub fn relocation_edits(
        &self,
        old: &Path,
        new: &Path,
        folder: bool,
        asset_root: Option<&Path>,
    ) -> Vec<(PathBuf, String, String)> {
        let moved = |path: &Path| {
            if folder {
                path.strip_prefix(old)
                    .map_or_else(|_| path.to_path_buf(), |suffix| new.join(suffix))
            } else if path == old {
                new.to_path_buf()
            } else {
                path.to_path_buf()
            }
        };
        let asset = |from: &Path, target: &str, wiki| {
            let root = asset_root?;
            crate::rendering::asset_path(
                root,
                &crate::rendering::Reference {
                    from: from.to_path_buf(),
                    target: target.into(),
                    wiki,
                },
                &self.files,
            )?
            .strip_prefix(root)
            .ok()
            .map(Path::to_path_buf)
        };
        let mut edits = vec![];
        for (from, note) in &self.notes {
            let source = moved(from);
            let mut replacements: Vec<(Range<usize>, String)> = vec![];
            for link in &note.parsed.links {
                if link.target.starts_with('#') {
                    continue;
                }
                let target = match self.resolve(from, &link.target) {
                    Resolution::Found(path) => path,
                    _ => match asset(from, &link.target, true) {
                        Some(path) => path,
                        None => continue,
                    },
                };
                let destination = moved(&target);
                if destination == target && source == *from {
                    continue;
                }
                let fragment = link
                    .target
                    .split_once('#')
                    .map_or(String::new(), |(_, s)| format!("#{s}"));
                let raw = &note.text[link.range.clone()];
                let alias = raw[2..raw.len() - 2]
                    .split_once('|')
                    .map_or(String::new(), |(_, s)| format!("|{s}"));
                let destination = if destination
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("md"))
                {
                    destination.with_extension("")
                } else {
                    destination
                };
                let target = destination.to_string_lossy().replace('\\', "/");
                replacements.push((
                    link.range.clone(),
                    format!("[[/{target}{fragment}{alias}]]"),
                ));
            }
            for (range, url) in &note.parsed.destinations {
                if url.starts_with('#') {
                    continue;
                }
                let target = match self.resolve_markdown(from, url).0 {
                    Resolution::Found(path) => path,
                    _ => match asset(from, url, false) {
                        Some(path) => path,
                        None => continue,
                    },
                };
                let destination = moved(&target);
                if destination == target && source == *from {
                    continue;
                }
                let parent = source.parent().unwrap_or(Path::new(""));
                let a: Vec<_> = parent.components().collect();
                let b: Vec<_> = destination.components().collect();
                let common = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
                let mut relative = PathBuf::new();
                for _ in common..a.len() {
                    relative.push("..");
                }
                for part in &b[common..] {
                    relative.push(part.as_os_str());
                }
                let path = relative.to_string_lossy().replace('\\', "/");
                let encoded =
                    percent_encoding::utf8_percent_encode(&path, LINK_PATH_ESCAPE).to_string();
                let fragment = url
                    .split_once('#')
                    .map_or(String::new(), |(_, s)| format!("#{s}"));
                replacements.push((range.clone(), format!("{encoded}{fragment}")));
            }
            if !replacements.is_empty() {
                replacements.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
                let mut after = note.text.clone();
                for (range, text) in replacements {
                    after.replace_range(range, &text);
                }
                if after != note.text {
                    edits.push((source, note.text.clone(), after));
                }
            }
        }
        edits
    }
    pub fn resolve(&self, from: &Path, target: &str) -> Resolution {
        self.resolve_literal(from, target.split('#').next().unwrap_or(""))
    }
    /// Markdown destinations are relative to the containing document, unlike wiki paths.
    /// Decode the path and fragment separately so an encoded '#' stays in the filename.
    pub fn resolve_markdown(&self, from: &Path, href: &str) -> (Resolution, Option<String>) {
        let (path, fragment) = href
            .split_once('#')
            .map_or((href, None), |(p, f)| (p, Some(f)));
        let Ok(path) = percent_encoding::percent_decode_str(path).decode_utf8() else {
            return (Resolution::Invalid, None);
        };
        let heading = match fragment.map(|f| percent_encoding::percent_decode_str(f).decode_utf8())
        {
            Some(Ok(text)) => Some(text.into_owned()),
            Some(Err(_)) => return (Resolution::Invalid, None),
            None => None,
        };
        if path.is_empty() {
            return (Resolution::Found(from.into()), heading);
        }
        if Path::new(path.as_ref())
            .extension()
            .is_some_and(|e| !e.eq_ignore_ascii_case("md"))
        {
            return (Resolution::Invalid, heading);
        }
        let target = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("./{path}")
        };
        let resolved = self.resolve_literal(from, &target);
        // Obsidian's shortest Markdown links may name a unique note elsewhere in the vault.
        let resolved = if matches!(resolved, Resolution::Missing(_)) && !path.contains('/') {
            self.resolve_literal(from, &path)
        } else {
            resolved
        };
        (resolved, heading)
    }
    fn resolve_literal(&self, from: &Path, target: &str) -> Resolution {
        let target = target.trim().replace('\\', "/");
        if target.is_empty() {
            return Resolution::Found(from.into());
        }
        if target.contains(':') {
            return Resolution::Invalid;
        }
        let explicit = target.contains('/');
        let mut path = if target.starts_with("./") || target.starts_with("../") {
            from.parent().unwrap_or(Path::new("")).join(&target)
        } else if explicit {
            PathBuf::from(target.trim_start_matches('/'))
        } else {
            from.parent().unwrap_or(Path::new("")).join(&target)
        };
        if path
            .extension()
            .is_none_or(|e| !e.eq_ignore_ascii_case("md"))
        {
            let mut name = path.into_os_string();
            name.push(".md");
            path = PathBuf::from(name);
        }
        let Some(path) = normalized(&path) else {
            return Resolution::Invalid;
        };
        if let Some(found) = self.notes.keys().find(|p| key(p) == key(&path)) {
            return Resolution::Found(found.clone());
        }
        if explicit {
            return Resolution::Missing(path);
        }
        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        let matches: Vec<_> = self
            .notes
            .keys()
            .filter(|p| {
                p.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase()
                    == stem
            })
            .cloned()
            .collect();
        match matches.len() {
            0 => {
                let aliases: Vec<_> = self
                    .notes
                    .iter()
                    .filter(|(_, n)| n.parsed.aliases.iter().any(|a| a.to_lowercase() == stem))
                    .map(|(p, _)| p.clone())
                    .collect();
                match aliases.len() {
                    0 => Resolution::Missing(path),
                    1 => Resolution::Found(aliases[0].clone()),
                    _ => Resolution::Ambiguous(aliases),
                }
            }
            1 => Resolution::Found(matches[0].clone()),
            _ => Resolution::Ambiguous(matches),
        }
    }
    pub fn backlinks(&self, target: &Path) -> Vec<PathBuf> {
        self.notes.iter().filter(|(from,note)| {
            note.parsed.links.iter().any(|l|matches!(self.resolve(from,&l.target),Resolution::Found(ref p)if key(p)==key(target))) ||
            note.parsed.standard_links.iter().any(|(url,_)|matches!(self.resolve_markdown(from,url).0,Resolution::Found(ref p)if key(p)==key(target)))
        }).map(|(p,_)|p.clone()).collect()
    }
}
