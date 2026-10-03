//! Portable linked-note bundles. All output is staged outside the source vault.
use super::*;
use crate::{
    index::{self, Index, Resolution},
    rendering::{self, Reference},
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Markdown,
    Html,
}
impl Format {
    fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Html => "html",
        }
    }
}
#[derive(Debug)]
pub struct Report {
    pub entry: PathBuf,
    pub notes: usize,
    pub attachments: usize,
    pub warnings: Vec<String>,
}
fn encoded(value: &str) -> String {
    // Encode every component separately, keeping directory separators portable.
    value
        .split('/')
        .map(|s| {
            percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
        })
        .collect::<Vec<_>>()
        .join("/")
}
fn relative_url(from: &Path, to: &Path) -> String {
    let a: Vec<_> = from
        .parent()
        .unwrap_or(Path::new(""))
        .components()
        .collect();
    let b: Vec<_> = to.components().collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut pieces = vec!["..".to_string(); a.len() - common];
    pieces.extend(
        b[common..]
            .iter()
            .map(|c| encoded(&c.as_os_str().to_string_lossy())),
    );
    pieces.join("/")
}
fn note_path(path: &Path, format: Format) -> PathBuf {
    Path::new("notes")
        .join(path)
        .with_extension(format.extension())
}
fn escape_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
}
fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
struct Builder<'a> {
    vault: &'a Vault,
    index: &'a Index,
    files: Vec<PathBuf>,
    format: Format,
    queue: VecDeque<PathBuf>,
    assets: BTreeSet<PathBuf>,
    warnings: BTreeSet<String>,
}
impl Builder<'_> {
    fn destination(&mut self, from: &Path, target: &str, wiki: bool) -> String {
        if rendering::is_external_link(target) {
            return target.to_owned();
        }
        if rendering::is_remote_image(target) {
            self.warnings.insert(format!(
                "{}：远程或内嵌资源未复制：{target}",
                from.display()
            ));
            return target.to_owned();
        }
        let (resolution, fragment) = if wiki {
            (
                self.index.resolve(from, target),
                target.split_once('#').map(|(_, f)| f.to_owned()),
            )
        } else {
            self.index.resolve_markdown(from, target)
        };
        if let Resolution::Found(path) = resolution
            && let Some(note) = self.index.notes.get(&path)
        {
            self.queue.push_back(path.clone());
            let mut url = relative_url(
                &note_path(from, self.format),
                &note_path(&path, self.format),
            );
            if let Some(fragment) = fragment.filter(|s| !s.is_empty()) {
                if self.format == Format::Markdown {
                    url.push('#');
                    url.push_str(&encoded(&fragment));
                } else if let Some(range) = index::anchor_range(&note.text, &note.parsed, &fragment)
                    && let Some(i) = note
                        .parsed
                        .headings
                        .iter()
                        .position(|h| h.offset == range.start)
                {
                    url.push_str(&format!("#heading-{i}"));
                } else {
                    self.warnings
                        .insert(format!("{}：未导出标题或块锚点：{target}", from.display()));
                }
            }
            return url;
        }
        let reference = Reference {
            from: from.into(),
            target: target.into(),
            wiki,
        };
        if let Some(asset) = rendering::asset_path(&self.vault.root, &reference, &self.files)
            && let Ok(path) = asset.strip_prefix(&self.vault.root)
            && self.files.iter().any(|p| p == path)
        {
            self.assets.insert(path.into());
            let mut url = relative_url(
                &note_path(from, self.format),
                &Path::new("assets").join(path),
            );
            if let Some((_, fragment)) = target.split_once('#') {
                url.push('#');
                url.push_str(&encoded(fragment));
            }
            return url;
        }
        self.warnings.insert(format!(
            "{}：无法解析或不允许的链接：{target}",
            from.display()
        ));
        // Do not leak vault-local file URLs or executable URI schemes into HTML.
        if self.format == Format::Html {
            "#unresolved-link".into()
        } else {
            encoded(target)
        }
    }
    fn markdown(&mut self, path: &Path, source: &str) -> String {
        let parsed = index::parse(source);
        let mut edits = vec![];
        for link in parsed.links {
            let mut range = link.range;
            let embed = range.start > 0
                && source.as_bytes()[range.start - 1] == b'!'
                && source[..range.start - 1]
                    .bytes()
                    .rev()
                    .take_while(|b| *b == b'\\')
                    .count()
                    % 2
                    == 0;
            if embed {
                range.start -= 1;
            }
            let url = self.destination(path, &link.target, true);
            let image = embed
                && Path::new(link.target.split('#').next().unwrap_or(""))
                    .extension()
                    .is_some_and(|e| {
                        matches!(
                            e.to_string_lossy().to_lowercase().as_str(),
                            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif"
                        )
                    });
            if embed && !image {
                self.warnings.insert(format!(
                    "{}：嵌入已转换为关联文件链接：{}",
                    path.display(),
                    link.target
                ));
            }
            edits.push((
                range,
                format!(
                    "{}[{}]({url})",
                    if image { "!" } else { "" },
                    escape_label(&link.label)
                ),
            ));
        }
        for (range, target) in parsed.destinations {
            edits.push((range, self.destination(path, &target, false)));
        }
        edits.sort_by_key(|(r, _)| r.start);
        let mut out = String::new();
        let mut cursor = 0;
        for (range, value) in edits {
            if range.start < cursor {
                continue;
            }
            out.push_str(&source[cursor..range.start]);
            out.push_str(&value);
            cursor = range.end;
        }
        out.push_str(&source[cursor..]);
        out
    }
}
fn html(title: &str, markdown: &str) -> io::Result<String> {
    let options = markdown_parser::Options {
        parse: crate::syntax::options(),
        compile: Default::default(),
    };
    let body = markdown_parser::to_html_with_options(markdown, &options)
        .map_err(|e| io::Error::other(e.to_string()))?;
    let heading = regex::Regex::new(r"<h([1-6])>").unwrap();
    let mut i = 0;
    let body = heading.replace_all(&body, |c: &regex::Captures<'_>| {
        let s = format!("<h{} id=\"heading-{i}\">", &c[1]);
        i += 1;
        s
    });
    Ok(format!(
        r#"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'">
<title>{}</title><style>
body{{max-width:820px;margin:40px auto;padding:0 24px;font:17px/1.75 system-ui,sans-serif;color:#242424;background:white;overflow-wrap:anywhere}}
img{{max-width:100%;height:auto}} pre{{white-space:pre-wrap;padding:16px;background:#f4f4f4}}code{{font-family:monospace}} table{{border-collapse:collapse;width:100%}}td,th{{border:1px solid #ccc;padding:6px 10px}}blockquote{{margin-left:0;padding-left:18px;border-left:3px solid #aaa;color:#555}}a{{color:#1664a2}}h1,h2,h3{{line-height:1.3}}.export-help{{font-size:13px;color:#666;border-bottom:1px solid #ddd;padding-bottom:12px}}
@media print{{body{{margin:0;max-width:none;font-size:11pt}}.export-help{{display:none}}h1,h2,h3{{break-after:avoid}}img,tr,pre{{break-inside:avoid}}@page{{margin:18mm}}}}
</style></head><body><aside class="export-help">墨砚 HTML 导出 · 使用浏览器“打印 → 另存为 PDF”。分享时请携带整个导出文件夹。远程图片不会自动加载。</aside><main>{body}</main></body></html>"#,
        html_escape(title)
    ))
}

/// `drafts` is a frozen snapshot of open editors, including unsaved changes.
pub fn create(
    vault: &Vault,
    entry: &Path,
    drafts: BTreeMap<PathBuf, String>,
    destination: &Path,
    format: Format,
) -> io::Result<Report> {
    let (stage, destination) = backup::staging(destination, &vault.root)?;
    let mut index = Index::build(vault).map_err(io::Error::other)?;
    for (path, text) in drafts {
        Vault::validate_relative(&path).map_err(io::Error::other)?;
        index.update(path, text);
    }
    if !index.notes.contains_key(entry) {
        return Err(io::Error::other("无法读取要导出的笔记"));
    }
    let mut builder = Builder {
        vault,
        index: &index,
        files: vault.scan_files().map_err(io::Error::other)?,
        format,
        queue: VecDeque::from([entry.into()]),
        assets: Default::default(),
        warnings: Default::default(),
    };
    let mut seen = BTreeSet::new();
    while let Some(path) = builder.queue.pop_front() {
        if !seen.insert(path.clone()) {
            continue;
        }
        let note = &index.notes[&path];
        let markdown = builder.markdown(&path, &note.text);
        let content = if format == Format::Html {
            html(
                &path.file_stem().unwrap_or_default().to_string_lossy(),
                &markdown,
            )?
        } else {
            markdown
        };
        let out = stage.0.join(note_path(&path, format));
        fs::create_dir_all(out.parent().unwrap())?;
        write_new_synced(&out, content.as_bytes())?;
    }
    for path in &builder.assets {
        let out = stage.0.join("assets").join(path);
        fs::create_dir_all(out.parent().unwrap())?;
        backup::transfer(&backup::safe_path(&vault.root, path)?, Some(&out))?;
    }
    if format == Format::Html {
        builder.warnings.insert("HTML 不执行原始 HTML 或脚本；公式、Mermaid、高亮等扩展暂以基本 Markdown 形式显示。块锚点不导出；远程图片不下载、不自动加载。".into());
    }
    let warnings: Vec<_> = builder.warnings.into_iter().collect();
    let report = format!(
        "墨砚导出\n入口：{}\n笔记：{}\n附件：{}\n包含当前编辑快照和递归关联笔记。嵌入笔记转换为链接。请携带整个目录。\n\n{}\n",
        note_path(entry, format).display(),
        seen.len(),
        builder.assets.len(),
        warnings.join("\n")
    );
    write_new_synced(&stage.0.join("导出说明.txt"), report.as_bytes())?;
    move_no_replace(&stage.0, &destination)?;
    Ok(Report {
        entry: destination.join(note_path(entry, format)),
        notes: seen.len(),
        attachments: builder.assets.len(),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_export_preserves_drafts_copies_assets_and_resolves_cycles() {
        let root = std::env::temp_dir().join(format!("inkstone-export-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        fs::create_dir(vault.root.join("目录")).unwrap();
        fs::create_dir(vault.root.join("附件")).unwrap();
        fs::write(vault.root.join("附件/中 文.png"), [0, 1, 2, 255]).unwrap();
        fs::write(vault.root.join("目录/a.md"), "disk").unwrap();
        fs::write(vault.root.join("b.md"), "# 标题\n[[目录/a]]\n").unwrap();
        let draft = "---\ntags: [test]\n---\n# 草稿 😀\r\n[[b#标题|别名]] ![[附件/中 文.png]]\r\n`[[not-a-link]]`\n";
        let report = create(
            &vault,
            Path::new("目录/a.md"),
            BTreeMap::from([("目录/a.md".into(), draft.into())]),
            &root.join("md"),
            Format::Markdown,
        )
        .unwrap();
        assert_eq!((report.notes, report.attachments), (2, 1));
        let text = fs::read_to_string(report.entry).unwrap();
        assert!(
            text.contains("[别名](../b%2Emd#%E6%A0%87%E9%A2%98)"),
            "{text}"
        );
        assert!(text.contains("`[[not-a-link]]`"));
        assert!(text.contains("tags: [test]"));
        assert_eq!(
            fs::read(root.join("md/assets/附件/中 文.png")).unwrap(),
            [0, 1, 2, 255]
        );
        assert_eq!(
            fs::read_to_string(vault.root.join("目录/a.md")).unwrap(),
            "disk"
        );
        let report = create(
            &vault,
            Path::new("目录/a.md"),
            BTreeMap::from([("目录/a.md".into(), draft.into())]),
            &root.join("html"),
            Format::Html,
        )
        .unwrap();
        let text = fs::read_to_string(report.entry).unwrap();
        assert!(text.contains("#heading-0"));
        assert!(text.contains("id=\"heading-0\""));
        assert!(!text.contains("tags: [test]"));
        assert!(
            create(
                &vault,
                Path::new("b.md"),
                BTreeMap::new(),
                &root.join("html"),
                Format::Html
            )
            .is_err()
        );
        assert!(
            create(
                &vault,
                Path::new("b.md"),
                BTreeMap::new(),
                &vault.root.join("nested"),
                Format::Html
            )
            .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn html_does_not_execute_source_markup_and_reports_missing_links() {
        let root = std::env::temp_dir().join(format!("inkstone-export-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        fs::write(vault.root.join("a.md"),"# <script>x</script>\n\n<script>alert(1)</script>\n\n[bad](javascript:alert) [[missing]]\n").unwrap();
        let report = create(
            &vault,
            Path::new("a.md"),
            BTreeMap::new(),
            &root.join("out"),
            Format::Html,
        )
        .unwrap();
        let html = fs::read_to_string(report.entry).unwrap();
        assert!(!html.contains("<script>"));
        assert!(!html.contains("href=\"javascript:"));
        assert!(report.warnings.iter().any(|s| s.contains("missing")));
        assert!(root.join("out/导出说明.txt").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
