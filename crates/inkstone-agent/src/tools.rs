use anyhow::{Context, Result, bail};
use inkstone_core::{
    changes::{Change, ChangeSet},
    index::Index,
    vault::Vault,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Scope {
    Notes(Vec<PathBuf>),
    Folder(PathBuf),
    Vault,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub path: PathBuf,
    pub hash: String,
    pub range: Range<usize>,
    pub unsaved: bool,
}
pub fn hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
fn relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().all(|c| matches!(c, Component::Normal(_)))
        && !path.to_string_lossy().contains(['\\', ':'])
        && !path
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
}
impl Scope {
    pub fn allows(&self, path: &Path, excluded: &[PathBuf]) -> bool {
        relative(path)
            && path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
            && !excluded.iter().any(|p| {
                PathBuf::from(path.to_string_lossy().to_lowercase())
                    .starts_with(p.to_string_lossy().to_lowercase())
            })
            && match self {
                Self::Notes(paths) => paths.iter().any(|p| p == path),
                Self::Folder(folder) => {
                    (folder.as_os_str().is_empty() || relative(folder)) && path.starts_with(folder)
                }
                Self::Vault => true,
            }
    }
}
/// Immutable per-run snapshot. All tool paths pass scope and Vault validation.
pub struct ToolHost {
    pub vault: Vault,
    pub scope: Scope,
    pub excluded: Vec<PathBuf>,
    pub index: Index,
    pub sources: Vec<Source>,
    pub source_prefix: String,
    pub proposals: Vec<ChangeSet>,
    overrides: BTreeMap<PathBuf, (String, Range<usize>)>,
    read_ranges: BTreeMap<PathBuf, Vec<Range<usize>>>,
}
impl ToolHost {
    pub fn new(
        vault: Vault,
        scope: Scope,
        excluded: Vec<PathBuf>,
        snapshot: Option<(PathBuf, String, Range<usize>)>,
    ) -> Result<Self> {
        let mut index = Index::default();
        let mut overrides = BTreeMap::new();
        if let Some((path, text, range)) = snapshot {
            if text.get(range.clone()).is_none() {
                bail!("选区已失效");
            }
            overrides.insert(path, (text, range));
        }
        for path in vault.scan_files()? {
            if !scope.allows(&path, &excluded) {
                continue;
            }
            let text = if let Some((text, range)) = overrides.get(&path) {
                text[range.clone()].to_owned()
            } else {
                vault.read(&path)?.unwrap_or_default()
            };
            if text.len() <= 4 * 1024 * 1024 {
                index.update(path, text);
            }
        }
        Ok(Self {
            vault,
            scope,
            excluded,
            index,
            sources: vec![],
            source_prefix: String::new(),
            proposals: vec![],
            overrides,
            read_ranges: BTreeMap::new(),
        })
    }
    fn fully_read(&self, path: &Path) -> bool {
        let Some(note) = self.index.notes.get(path) else {
            return false;
        };
        let Some(ranges) = self.read_ranges.get(path) else {
            return false;
        };
        let mut ranges = ranges.clone();
        ranges.sort_by_key(|r| r.start);
        let mut end = 0;
        for range in ranges {
            if range.start > end {
                return false;
            }
            end = end.max(range.end);
        }
        end >= note.text.len()
    }
    fn check(&self, path: &Path) -> Result<()> {
        if !self.scope.allows(path, &self.excluded) {
            bail!("路径不在本次任务允许范围内");
        }
        self.vault.path(path)?;
        Ok(())
    }
    fn cite(&mut self, path: &Path, range: Range<usize>) -> Result<Source> {
        self.check(path)?;
        let note = self.index.notes.get(path).context("笔记不存在")?;
        let (text, range, unsaved) = if let Some((text, selection)) = self.overrides.get(path) {
            (
                text.as_str(),
                range.start + selection.start..range.end + selection.start,
                true,
            )
        } else {
            (note.text.as_str(), range, false)
        };
        let source = Source {
            id: format!("S{}{}", self.source_prefix, self.sources.len() + 1),
            path: path.into(),
            hash: hash(text),
            range,
            unsaved,
        };
        self.sources.push(source.clone());
        Ok(source)
    }
    pub fn call(&mut self, name: &str, args: Value) -> Result<Value> {
        let offset = args["offset"].as_u64().unwrap_or(0).min(1_000_000) as usize;
        let limit = args["limit"].as_u64().unwrap_or(20).clamp(1, 50) as usize;
        match name {
            "list_notes" => {
                let paths: Vec<_> = self.index.notes.keys().skip(offset).take(limit).collect();
                Ok(
                    json!({"paths":paths,"next_offset":offset + paths.len(),"total":self.index.notes.len()}),
                )
            }
            "search_notes" => {
                let query = args["query"].as_str().context("缺少 query")?;
                let hits = self
                    .index
                    .search_limited(query, false, Default::default(), false, offset + limit)
                    .map_err(anyhow::Error::msg)?;
                let mut result = vec![];
                for hit in hits.into_iter().skip(offset).take(limit) {
                    let source = self.cite(&hit.path, hit.offset..hit.offset)?;
                    result.push(json!({"source":source,"excerpt":hit.excerpt}));
                }
                let titles: Vec<_> = self
                    .index
                    .filenames(query)
                    .into_iter()
                    .take(10)
                    .map(|h| h.path)
                    .collect();
                Ok(json!({"matches":result,"titles":titles,"next_offset":offset + result.len()}))
            }
            "read_note" | "outline" | "links" => {
                let path = PathBuf::from(args["path"].as_str().context("缺少 path")?);
                self.check(&path)?;
                let note = self
                    .index
                    .notes
                    .get(&path)
                    .context("笔记不存在或过大")?
                    .clone();
                if name == "outline" {
                    return Ok(
                        json!({"headings":note.parsed.headings.iter().skip(offset).take(limit).collect::<Vec<_>>()}),
                    );
                }
                if name == "links" {
                    let links: Vec<_> = note
                        .parsed
                        .links
                        .iter()
                        .filter_map(|link| match self.index.resolve(&path, &link.target) {
                            inkstone_core::index::Resolution::Found(target)
                                if self.scope.allows(&target, &self.excluded) =>
                            {
                                Some(target)
                            }
                            _ => None,
                        })
                        .skip(offset)
                        .take(limit)
                        .collect();
                    return Ok(
                        json!({"links":links,"backlinks":self.index.backlinks(&path).into_iter().filter(|p| self.scope.allows(p, &self.excluded)).skip(offset).take(limit).collect::<Vec<_>>()}),
                    );
                }
                let start = args["start"]
                    .as_u64()
                    .unwrap_or(0)
                    .min(note.text.len() as u64) as usize;
                let mut end = (start + 12000).min(note.text.len());
                if let Some(value) = args["end"].as_u64() {
                    end = end.min(value as usize);
                }
                if end < start || !note.text.is_char_boundary(start) {
                    bail!("无效的 UTF-8 源码范围");
                }
                while !note.text.is_char_boundary(end) {
                    end -= 1;
                }
                let source = self.cite(&path, start..end)?;
                self.read_ranges.entry(path).or_default().push(start..end);
                Ok(
                    json!({"source":source,"text":&note.text[start..end],"next_start":end,"total_bytes":note.text.len()}),
                )
            }
            "propose_changes" => {
                let edits = args["changes"].as_array().context("缺少 changes")?;
                if edits.is_empty() || edits.len() > 100 {
                    bail!("一次最多提出 100 篇修改");
                }
                let mut set = ChangeSet::default();
                let mut paths = BTreeSet::new();
                for edit in edits {
                    let path = PathBuf::from(edit["path"].as_str().context("缺少 path")?);
                    self.check(&path)?;
                    if !paths.insert(path.to_string_lossy().to_lowercase()) {
                        bail!("方案中路径重复");
                    }
                    let mut after = edit["content"]
                        .as_str()
                        .context("缺少完整 content")?
                        .to_owned();
                    let before = if let Some((text, range)) = self.overrides.get(&path) {
                        if !self.fully_read(&path) {
                            bail!("必须先完整读取目标笔记（使用 next_start 继续读取）");
                        }
                        let mut full = text.clone();
                        full.replace_range(range.clone(), &after);
                        after = full;
                        Some(text.clone())
                    } else if let Some(note) = self.index.notes.get(&path) {
                        if !self.fully_read(&path) {
                            bail!("必须先完整读取目标笔记（使用 next_start 继续读取）");
                        }
                        Some(note.text.clone())
                    } else {
                        if self.vault.read(&path)?.is_some() {
                            bail!("目标文件存在但未读取");
                        }
                        None
                    };
                    if after.len() > 1024 * 1024 {
                        bail!("单篇提案超过 1 MiB");
                    }
                    set.changes.push(Change {
                        path,
                        before,
                        after,
                        reason: edit["reason"].as_str().unwrap_or("Agent 修改提案").into(),
                    });
                }
                // One proposal is one dependency group: accepting it cannot omit a
                // newly-created note referenced by another change in the same group.
                set.sources = self.sources.iter().map(|s| s.id.clone()).collect();
                self.proposals.push(set);
                Ok(json!({"proposal":self.proposals.len(),"status":"等待用户审阅，尚未修改文件"}))
            }
            _ => bail!("未知工具"),
        }
    }
}
pub fn definitions() -> Vec<Value> {
    let entries = [
        (
            "list_notes",
            "分页列出授权范围内笔记",
            json!({"offset":{"type":"integer"},"limit":{"type":"integer"}}),
            vec![],
        ),
        (
            "search_notes",
            "全文、标签与文件名检索；用不同关键词多轮检索",
            json!({"query":{"type":"string"},"offset":{"type":"integer"},"limit":{"type":"integer"}}),
            vec!["query"],
        ),
        (
            "read_note",
            "读取笔记 UTF-8 字节范围，单次最多 12000 字节，继续使用 next_start",
            json!({"path":{"type":"string"},"start":{"type":"integer"},"end":{"type":"integer"}}),
            vec!["path"],
        ),
        (
            "outline",
            "分页查看大纲",
            json!({"path":{"type":"string"},"offset":{"type":"integer"},"limit":{"type":"integer"}}),
            vec!["path"],
        ),
        (
            "links",
            "分页查看双链及反链",
            json!({"path":{"type":"string"},"offset":{"type":"integer"},"limit":{"type":"integer"}}),
            vec!["path"],
        ),
        (
            "propose_changes",
            "提出一组相互依赖的修改供用户审阅，绝不会写入文件。content 为完整新正文；当前选区则仅为新选区内容。先读后改；独立修改分别提案。",
            json!({"changes":{"type":"array","items":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"},"reason":{"type":"string"}},"required":["path","content","reason"],"additionalProperties":false}}}),
            vec!["changes"],
        ),
    ];
    entries.into_iter().map(|(name, description, properties, required)| json!({"type":"function","function":{"name":name,"description":description,"parameters":{"type":"object","properties":properties,"required":required,"additionalProperties":false}}})).collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_blocks_escape_and_exclusions() {
        let s = Scope::Folder("work".into());
        for path in [
            "../work/a.md",
            "/work/a.md",
            "work/../a.md",
            "work/private/a.md",
            "work/.secret.md",
            "work/a.txt",
        ] {
            assert!(!s.allows(Path::new(path), &["work/private".into()]));
        }
        assert!(s.allows(Path::new("work/a.md"), &[]));
    }
    #[test]
    fn selection_cannot_read_surroundings_and_proposal_never_writes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        std::fs::create_dir(&root).unwrap();
        let vault = Vault::open(&root, dir.path().join("history")).unwrap();
        vault.create(Path::new("a.md"), "secret中文tail").unwrap();
        let mut host = ToolHost::new(
            vault.clone(),
            Scope::Notes(vec!["a.md".into()]),
            vec![],
            Some(("a.md".into(), "secret中文tail".into(), 6..12)),
        )
        .unwrap();
        let read = host.call("read_note", json!({"path":"a.md"})).unwrap();
        assert_eq!(read["text"], "中文");
        assert_eq!(read["source"]["range"]["start"], 6);
        host.call(
            "propose_changes",
            json!({"changes":[{"path":"a.md","content":"修改","reason":"test"}]}),
        )
        .unwrap();
        assert_eq!(host.proposals[0].changes[0].after, "secret修改tail");
        assert_eq!(
            vault.read(Path::new("a.md")).unwrap().unwrap(),
            "secret中文tail"
        );
        assert!(host.call("read_note", json!({"path":"../a.md"})).is_err());
    }
    #[test]
    fn partial_read_cannot_replace_full_note() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        std::fs::create_dir(&root).unwrap();
        let vault = Vault::open(&root, dir.path().join("history")).unwrap();
        vault.create(Path::new("a.md"), &"a".repeat(13000)).unwrap();
        let mut host = ToolHost::new(vault, Scope::Vault, vec![], None).unwrap();
        host.call("read_note", json!({"path":"a.md"})).unwrap();
        let proposal = json!({"changes":[{"path":"a.md","content":"new","reason":"test"}]});
        assert!(host.call("propose_changes", proposal.clone()).is_err());
        host.call("read_note", json!({"path":"a.md","start":12000}))
            .unwrap();
        assert!(host.call("propose_changes", proposal).is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_read_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(dir.path().join("outside.md"), "secret").unwrap();
        std::os::unix::fs::symlink(dir.path().join("outside.md"), root.join("alias.md")).unwrap();
        let vault = Vault::open(&root, dir.path().join("history")).unwrap();
        let mut host = ToolHost::new(vault, Scope::Vault, vec![], None).unwrap();
        assert!(host.call("read_note", json!({"path":"alias.md"})).is_err());
    }
}
