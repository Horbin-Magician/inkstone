//! Portable linked-note bundles. All output is staged outside the source vault.
use super::*;
use crate::{
    index::{self, Index, Resolution},
    rendering::{self, Reference},
};
use std::{
    cell::OnceCell,
    collections::{BTreeMap, BTreeSet, HashSet, VecDeque},
};

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
    asset_catalog: OnceCell<HashSet<&'a Path>>,
    format: Format,
    queue: VecDeque<PathBuf>,
    scheduled: BTreeSet<PathBuf>,
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
            if self.scheduled.insert(path.clone()) {
                self.queue.push_back(path.clone());
            }
            let mut url = relative_url(
                &note_path(from, self.format),
                &note_path(&path, self.format),
            );
            if let Some(fragment) = fragment.filter(|s| !s.is_empty()) {
                let anchor = if wiki {
                    index::anchor_range
                } else {
                    index::anchor_range_decoded
                };
                if self.format == Format::Markdown {
                    url.push('#');
                    url.push_str(&encoded(&fragment));
                } else if let Some(id) = fragment.strip_prefix('^')
                    && let Some(i) = note.parsed.blocks.iter().position(|b| b.id == id)
                {
                    url.push_str(&format!("#block-{i}"));
                } else if let Some(range) = anchor(&note.text, &note.parsed, &fragment)
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
        if let Some(asset) = rendering::asset_path(&self.vault.root, &reference, &self.index.files)
            && let Ok(path) = asset.strip_prefix(&self.vault.root)
            && self
                .asset_catalog
                .get_or_init(|| self.index.files.iter().map(PathBuf::as_path).collect())
                .contains(path)
        {
            self.assets.insert(path.into());
            let mut url = relative_url(
                &note_path(from, self.format),
                &Path::new("assets").join(path),
            );
            // Markdown fragments are decoded by resolve_markdown; Wiki fragments
            // are literal. Preserve the previous fallback for malformed escapes.
            if let Some(fragment) = fragment
                .as_deref()
                .or_else(|| target.split_once('#').map(|(_, value)| value))
            {
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
    fn markdown(&mut self, path: &Path, note: &index::IndexedNote) -> String {
        let source = &note.text;
        let parsed = &note.parsed;
        let mut edits = vec![];
        for link in &parsed.links {
            let mut range = link.range.clone();
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
        for embed in &parsed.markdown_embeds {
            let url = self.destination(path, &embed.target, false);
            self.warnings.insert(format!(
                "{}：嵌入已转换为关联文件链接：{}",
                path.display(),
                embed.target
            ));
            edits.push((
                embed.range.clone(),
                format!("[{}]({url})", escape_label(&embed.label)),
            ));
        }
        for (range, target) in &parsed.destinations {
            edits.push((range.clone(), self.destination(path, target, false)));
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
fn html_extensions(source: &str, warnings: &mut BTreeSet<String>) -> (String, String, Vec<String>) {
    use crate::graphics::{self, Kind};
    use markdown_parser::mdast::Node;
    fn visit(node: &Node, out: &mut Vec<(std::ops::Range<usize>, Kind, String)>) {
        let graphic = match node {
            Node::InlineMath(n) => Some((Kind::InlineMath, n.value.clone())),
            Node::Math(n) => Some((Kind::BlockMath, n.value.clone())),
            Node::Code(n)
                if n.lang
                    .as_deref()
                    .is_some_and(|s| s.eq_ignore_ascii_case("mermaid")) =>
            {
                Some((Kind::Mermaid, n.value.clone()))
            }
            _ => None,
        };
        if let Some((kind, text)) = graphic
            && let Some(p) = node.position()
        {
            out.push((p.start.offset..p.end.offset, kind, text));
        } else if let Some(children) = node.children() {
            for child in children {
                visit(child, out);
            }
        }
    }
    let snapshot = crate::syntax::Snapshot::new(source);
    let mut graphics = vec![];
    if let Some(ast) = &snapshot.ast {
        visit(ast, &mut graphics);
    }
    let mut edits = vec![];
    if graphics.len() > 200 {
        warnings.insert("单篇公式/图表超过 200 个，超出部分保留源码。".into());
    }
    for (range, kind, text) in graphics.into_iter().take(200) {
        match graphics::prepare(kind, &text, true) {
            Ok(graphic) => {
                let svg = String::from_utf8_lossy(&graphic.svg);
                let data =
                    percent_encoding::utf8_percent_encode(&svg, percent_encoding::NON_ALPHANUMERIC);
                let class = if kind == Kind::InlineMath {
                    "inline-math"
                } else {
                    "block-graphic"
                };
                edits.push((range, format!("<img class=\"{class}\" alt=\"{}\" width=\"{}\" height=\"{}\" src=\"data:image/svg+xml,{data}\">", html_escape(&text), graphic.width.ceil(), graphic.height.ceil())));
            }
            Err(error) => {
                warnings.insert(format!("公式或图表保留源码：{error}"));
            }
        }
    }
    for span in crate::markdown::spans_snapshot(&snapshot)
        .into_iter()
        .filter(|s| s.kind == crate::markdown::Kind::Highlight)
    {
        edits.push((span.markers[0].clone(), "<mark>".into()));
        edits.push((span.markers[1].clone(), "</mark>".into()));
    }
    for (i, block) in index::parse_snapshot(&snapshot)
        .blocks
        .into_iter()
        .enumerate()
    {
        edits.push((block.marker, format!("<span id=\"block-{i}\"></span>")));
    }
    edits.sort_by_key(|(range, _)| (range.start, std::cmp::Reverse(range.end)));
    let prefix = loop {
        let prefix = format!("INKSTONEEXPORT{}TOKEN", unique_id().replace('-', ""));
        if !source.contains(&prefix) {
            break prefix;
        }
    };
    let mut out = String::new();
    let mut tokens = vec![];
    let mut cursor = 0;
    for (range, markup) in edits {
        if range.start < cursor {
            continue;
        }
        out.push_str(&snapshot.structural[cursor..range.start]);
        let token = format!("{prefix}{}END", tokens.len());
        out.push_str(&token);
        tokens.push(markup);
        cursor = range.end;
    }
    out.push_str(&snapshot.structural[cursor..]);
    (out, prefix, tokens)
}
fn restore_html_tokens(body: String, prefix: &str, tokens: &[String]) -> String {
    if tokens.is_empty() {
        return body;
    }
    let mut out = String::with_capacity(body.len());
    let mut remaining = body.as_str();
    while let Some(start) = remaining.find(prefix) {
        out.push_str(&remaining[..start]);
        let after_prefix = &remaining[start + prefix.len()..];
        let digits = after_prefix.bytes().take_while(u8::is_ascii_digit).count();
        let number = &after_prefix[..digits];
        let replacement = (!number.is_empty()
            && (number.len() == 1 || !number.starts_with('0'))
            && after_prefix[digits..].starts_with("END"))
        .then(|| number.parse::<usize>().ok().and_then(|i| tokens.get(i)))
        .flatten();
        if let Some(markup) = replacement {
            out.push_str(markup);
            remaining = &after_prefix[digits + 3..];
        } else {
            out.push_str(prefix);
            remaining = after_prefix;
        }
    }
    out.push_str(remaining);
    out
}

fn html(title: &str, markdown: &str, warnings: &mut BTreeSet<String>) -> io::Result<String> {
    let (markdown, prefix, tokens) = html_extensions(markdown, warnings);
    let options = markdown_parser::Options {
        parse: crate::syntax::options(),
        compile: Default::default(),
    };
    let body = markdown_parser::to_html_with_options(&markdown, &options)
        .map_err(|e| io::Error::other(e.to_string()))?;
    let body = restore_html_tokens(body, &prefix, &tokens);
    static HEADING: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"<h([1-6])>").unwrap());
    let mut i = 0;
    let body = HEADING.replace_all(&body, |c: &regex::Captures<'_>| {
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
.inline-math{{vertical-align:middle;display:inline}}.block-graphic{{display:block;margin:1em auto}}mark{{background:#fff0a0;color:inherit}}
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
    for path in drafts.keys() {
        Vault::validate_relative(path).map_err(io::Error::other)?;
    }
    index.update_many(drafts);
    if !index.notes.contains_key(entry) {
        return Err(io::Error::other("无法读取要导出的笔记"));
    }
    let mut builder = Builder {
        vault,
        index: &index,
        asset_catalog: OnceCell::new(),
        format,
        queue: VecDeque::from([entry.into()]),
        scheduled: BTreeSet::from([entry.into()]),
        assets: Default::default(),
        warnings: Default::default(),
    };
    while let Some(path) = builder.queue.pop_front() {
        let note = &index.notes[&path];
        let markdown = builder.markdown(&path, note);
        let content = if format == Format::Html {
            html(
                &path.file_stem().unwrap_or_default().to_string_lossy(),
                &markdown,
                &mut builder.warnings,
            )?
        } else {
            markdown
        };
        let relative_output = note_path(&path, format);
        let out = stage.0.join(&relative_output);
        fs::create_dir_all(out.parent().unwrap())
            .and_then(|()| write_new_synced(&out, content.as_bytes()))
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!(
                        "无法导出笔记“{}”到“{}”：{error}",
                        path.display(),
                        relative_output.display()
                    ),
                )
            })?;
    }
    for path in &builder.assets {
        let out = stage.0.join("assets").join(path);
        fs::create_dir_all(out.parent().unwrap())?;
        backup::transfer(&backup::safe_path(&vault.root, path)?, Some(&out))?;
    }
    if format == Format::Html {
        builder.warnings.insert("HTML 不执行原始 HTML 或脚本；公式与 Mermaid 使用本地 SVG 渲染，失败时保留源码并列出提示。远程图片不下载、不自动加载。".into());
    }
    let warnings: Vec<_> = builder.warnings.into_iter().collect();
    let report = format!(
        "墨砚导出\n入口：{}\n笔记：{}\n附件：{}\n包含当前编辑快照和递归关联笔记。嵌入笔记转换为链接。请携带整个目录。\n\n{}\n",
        note_path(entry, format).display(),
        builder.scheduled.len(),
        builder.assets.len(),
        warnings.join("\n")
    );
    write_new_synced(&stage.0.join("导出说明.txt"), report.as_bytes())?;
    move_no_replace(&stage.0, &destination)?;
    Ok(Report {
        entry: destination.join(note_path(entry, format)),
        notes: builder.scheduled.len(),
        attachments: builder.assets.len(),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_restoration_matches_literal_replacement_for_repeated_and_invalid_tokens() {
        let prefix = "INKSTONEEXPORTTESTTOKEN";
        let tokens: Vec<_> = (0..256)
            .map(|i| format!("<mark>中文😀{i}</mark>"))
            .collect();
        let mut body = "前文 &amp; 👩‍💻".repeat(1_000);
        for i in (0..tokens.len()).rev().chain(0..tokens.len()) {
            body.push_str(&format!("{prefix}{i}END"));
        }
        for suffix in [
            "",
            "END",
            "-1END",
            "01END",
            "256END",
            "1EN",
            "999999999999999999999999999999END",
        ] {
            body.push_str(&format!("<p>{prefix}{suffix}</p>"));
        }
        let mut expected = body.clone();
        for (i, markup) in tokens.iter().enumerate() {
            expected = expected.replace(&format!("{prefix}{i}END"), markup);
        }
        assert_eq!(restore_html_tokens(body, prefix, &tokens), expected);
        assert_eq!(
            restore_html_tokens("unchanged".into(), prefix, &[]),
            "unchanged"
        );
    }

    #[test]
    fn formatted_wiki_aliases_keep_inline_styles_and_portable_targets() {
        let root = std::env::temp_dir().join(format!("inkstone-export-alias-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let source = "[[target|**粗体** 与 *斜体* 与 ~~删除~~]]\n";
        fs::write(vault.root.join("source.md"), source).unwrap();
        fs::write(vault.root.join("target.md"), "# Target").unwrap();
        for format in [Format::Markdown, Format::Html] {
            let report = create(
                &vault,
                Path::new("source.md"),
                BTreeMap::new(),
                &root.join(format.extension()),
                format,
            )
            .unwrap();
            assert_eq!(report.notes, 2);
            let output = fs::read_to_string(report.entry).unwrap();
            match format {
                Format::Markdown => assert!(
                    output.contains("[**粗体** 与 *斜体* 与 ~~删除~~](target%2Emd)"),
                    "{output}"
                ),
                Format::Html => {
                    assert!(output.contains("href=\"target%2Ehtml\""), "{output}");
                    for markup in ["<strong>粗体</strong>", "<em>斜体</em>", "<del>删除</del>"]
                    {
                        assert!(output.contains(markup), "{output}");
                    }
                }
            }
        }
        assert_eq!(
            fs::read_to_string(vault.root.join("source.md")).unwrap(),
            source
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn attachment_catalog_still_excludes_files_outside_the_index() {
        let root = std::env::temp_dir().join(format!("inkstone-export-catalog-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        fs::write(vault.root.join("allowed.bin"), b"allowed").unwrap();
        fs::write(vault.root.join("excluded.bin"), b"excluded").unwrap();
        let mut index = Index::default();
        index.files.push("allowed.bin".into());
        let mut builder = Builder {
            vault: &vault,
            index: &index,
            asset_catalog: OnceCell::new(),
            format: Format::Html,
            queue: Default::default(),
            scheduled: Default::default(),
            assets: Default::default(),
            warnings: Default::default(),
        };
        assert_eq!(
            builder.destination(Path::new("note.md"), "excluded.bin", false),
            "#unresolved-link"
        );
        assert!(builder.assets.is_empty());
        for _ in 0..10 {
            assert_eq!(
                builder.destination(Path::new("note.md"), "allowed.bin", false),
                "../assets/allowed%2Ebin"
            );
        }
        assert_eq!(
            builder.assets,
            BTreeSet::from([PathBuf::from("allowed.bin")])
        );
        assert_eq!(
            fs::read(vault.root.join("excluded.bin")).unwrap(),
            b"excluded"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn html_path_collisions_report_the_note_and_leave_no_partial_output() {
        let root = std::env::temp_dir().join(format!("inkstone-export-collision-{}", unique_id()));
        fs::create_dir_all(root.join("vault/a.html")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let first = "[child](a.html/b.md)";
        let second = "[parent](../a.md)";
        fs::write(vault.root.join("a.md"), first).unwrap();
        fs::write(vault.root.join("a.html/b.md"), second).unwrap();
        let children = || -> BTreeSet<PathBuf> {
            fs::read_dir(&root)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect()
        };
        let before = children();
        for (entry, failed_note, failed_output) in [
            ("a.md", "a.html/b.md", "notes/a.html/b.html"),
            ("a.html/b.md", "a.md", "notes/a.html"),
        ] {
            let destination = root.join("export");
            let error = create(
                &vault,
                Path::new(entry),
                BTreeMap::new(),
                &destination,
                Format::Html,
            )
            .unwrap_err();
            let message = error.to_string();
            assert!(
                message.contains(&failed_note.replace('/', std::path::MAIN_SEPARATOR_STR)),
                "{message}"
            );
            assert!(
                message.contains(&failed_output.replace('/', std::path::MAIN_SEPARATOR_STR)),
                "{message}"
            );
            assert!(!destination.exists());
            assert_eq!(children(), before);
            assert_eq!(fs::read_to_string(vault.root.join("a.md")).unwrap(), first);
            assert_eq!(
                fs::read_to_string(vault.root.join("a.html/b.md")).unwrap(),
                second
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repeated_and_self_references_schedule_each_note_once() {
        let mut index = Index::default();
        index.update("a.md".into(), "# A".into());
        index.update("b.md".into(), "# B".into());
        let vault = Vault {
            root: PathBuf::new(),
            recovery_dir: PathBuf::new(),
        };
        let mut builder = Builder {
            vault: &vault,
            index: &index,
            asset_catalog: OnceCell::new(),
            format: Format::Markdown,
            queue: VecDeque::from(["a.md".into()]),
            scheduled: BTreeSet::from(["a.md".into()]),
            assets: Default::default(),
            warnings: Default::default(),
        };
        for _ in 0..1_000 {
            assert_eq!(builder.destination(Path::new("a.md"), "b", true), "b%2Emd");
            assert_eq!(builder.destination(Path::new("a.md"), "a", true), "a%2Emd");
        }
        assert_eq!(
            builder.queue,
            VecDeque::from([PathBuf::from("a.md"), PathBuf::from("b.md")])
        );
        builder.queue.clear();
        builder.destination(Path::new("b.md"), "a.md", false);
        assert!(
            builder.queue.is_empty(),
            "completed notes must not be scheduled again"
        );
    }
    #[test]
    fn attachment_fragments_are_encoded_once_in_portable_exports() {
        let root = std::env::temp_dir().join(format!("inkstone-export-fragments-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let asset = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
        fs::write(vault.root.join("asset.svg"), asset).unwrap();
        let source = "[space](asset.svg#part%20one)\n\n[percent](asset.svg#part%2520one)\n\n![image](asset.svg#%E4%B8%AD%E6%96%87)\n\n[[asset.svg#literal%20value|wiki]]\n\n[invalid](asset.svg#%FF)\n";
        fs::write(vault.root.join("note.md"), source).unwrap();
        for format in [Format::Markdown, Format::Html] {
            let output = root.join(format.extension());
            let report = create(
                &vault,
                Path::new("note.md"),
                BTreeMap::new(),
                &output,
                format,
            )
            .unwrap();
            assert_eq!((report.notes, report.attachments), (1, 1));
            let exported = fs::read_to_string(report.entry).unwrap();
            for fragment in [
                "part%20one",
                "part%2520one",
                "%E4%B8%AD%E6%96%87",
                "literal%2520value",
                "%25FF",
            ] {
                assert!(
                    exported.contains(&format!("../assets/asset%2Esvg#{fragment}")),
                    "{exported}"
                );
            }
            assert_eq!(fs::read(output.join("assets/asset.svg")).unwrap(), asset);
        }
        assert_eq!(
            fs::read_to_string(vault.root.join("note.md")).unwrap(),
            source
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn relocated_wiki_assets_keep_rendering_and_portable_resources() {
        let root =
            std::env::temp_dir().join(format!("inkstone-export-renamed-assets-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        for (i, (extension, new_extension, alias)) in [
            ("png", "png", "160x80"),
            ("png", "png", "图示"),
            ("pdf", "pdf", "说明"),
            ("png", "pdf", "改为附件"),
        ]
        .into_iter()
        .enumerate()
        {
            let old = PathBuf::from(format!("old{i}.{extension}"));
            let new = PathBuf::from(format!("new#[{i}].{new_extension}"));
            fs::write(vault.root.join(&old), b"asset fixture").unwrap();
            fs::write(
                vault.root.join("root.md"),
                format!("![[{}|{alias}]]", old.display()),
            )
            .unwrap();
            let index = Index::build(&vault).unwrap();
            let edits = index.relocation_edits(&old, &new, false, Some(&vault.root));
            let after = &edits
                .iter()
                .find(|(path, _, _)| path == Path::new("root.md"))
                .unwrap()
                .2;
            fs::rename(vault.root.join(&old), vault.root.join(&new)).unwrap();
            fs::write(vault.root.join("root.md"), after).unwrap();
            let index = Index::build(&vault).unwrap();
            let reading = rendering::reading_document(&index, Path::new("root.md"), after);
            assert_eq!(
                rendering::asset_path(
                    &vault.root,
                    &reading.references[0],
                    std::slice::from_ref(&new)
                ),
                Some(vault.root.join(&new))
            );
            if alias == "160x80" {
                assert!(
                    reading.markdown.contains("width=\"160\" height=\"80\""),
                    "{}",
                    reading.markdown
                );
            } else if new_extension == "pdf" {
                assert!(reading.markdown.contains(&format!("附件：{alias}")));
                assert!(!after.contains("![["));
            }
            for (format, directory) in [(Format::Markdown, "md"), (Format::Html, "html")] {
                let output = root.join(format!("{directory}-{i}"));
                let report = create(
                    &vault,
                    Path::new("root.md"),
                    BTreeMap::new(),
                    &output,
                    format,
                )
                .unwrap();
                assert_eq!(report.attachments, 1);
                assert_eq!(
                    fs::read(output.join("assets").join(&new)).unwrap(),
                    b"asset fixture"
                );
                let text = fs::read_to_string(report.entry).unwrap();
                assert!(text.contains("assets/new%23%5B"), "{text}");
                if format == Format::Markdown && alias == "160x80" {
                    assert!(text.contains("\\|160x80"));
                }
            }
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn markdown_percent_encoded_heading_is_decoded_once() {
        let root =
            std::env::temp_dir().join(format!("inkstone-export-percent-heading-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        fs::write(
            vault.root.join("child.md"),
            "# A\nwrong\n\n# %41\ncorrect\n",
        )
        .unwrap();
        fs::write(vault.root.join("root.md"), "[literal](child.md#%2541)").unwrap();
        for (format, directory, fragment) in [
            (Format::Markdown, "md", "#%2541"),
            (Format::Html, "html", "#heading-1"),
        ] {
            let report = create(
                &vault,
                Path::new("root.md"),
                BTreeMap::new(),
                &root.join(directory),
                format,
            )
            .unwrap();
            assert!(
                report
                    .warnings
                    .iter()
                    .all(|warning| warning.starts_with("HTML 不执行原始 HTML 或脚本；")),
                "{:?}",
                report.warnings
            );
            let output = fs::read_to_string(report.entry).unwrap();
            assert!(output.contains(fragment), "{output}");
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn markdown_note_embeds_export_as_portable_links() {
        let root = std::env::temp_dir().join(format!("inkstone-export-embeds-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        fs::write(vault.root.join("target#note.md"), "# Section\n正文").unwrap();
        fs::write(
            vault.root.join("root.md"),
            "![片段](target%23note.md#Section)\n\n![引用][r]\n\n[r]: target%23note.md#Section",
        )
        .unwrap();
        for (format, directory) in [(Format::Markdown, "md"), (Format::Html, "html")] {
            let report = create(
                &vault,
                Path::new("root.md"),
                BTreeMap::new(),
                &root.join(directory),
                format,
            )
            .unwrap();
            assert_eq!(report.notes, 2);
            assert_eq!(report.attachments, 0);
            assert!(report.warnings.iter().any(|w| w.contains("嵌入已转换")));
            let output = fs::read_to_string(report.entry).unwrap();
            assert!(!output.contains("![片段]"), "{output}");
            assert!(!output.contains("<img"), "{output}");
            assert!(output.contains("target%23note%2E"), "{output}");
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn html_exports_native_graphics_highlights_and_block_anchors() {
        let root = std::env::temp_dir().join(format!("inkstone-export-rich-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        fs::write(vault.root.join("a.md"), "# Rich\n\n$\\frac{1}{2}$ and ==重点== `==code==`\n\n```mermaid\nflowchart LR\nA --> B\n```\n\n正文 ^ref\n\n[[a#^ref]]\n\n%%private comment%%\n").unwrap();
        let report = create(
            &vault,
            Path::new("a.md"),
            BTreeMap::new(),
            &root.join("out"),
            Format::Html,
        )
        .unwrap();
        let html = fs::read_to_string(report.entry).unwrap();
        assert_eq!(html.matches("src=\"data:image/svg+xml,").count(), 2);
        assert!(html.contains("<mark>重点</mark>"));
        assert!(html.contains("<code>==code==</code>"));
        assert!(html.contains("id=\"block-0\""));
        assert!(html.contains("#block-0\""));
        assert!(!html.contains("private comment"));
        assert!(!html.contains("INKSTONEEXPORT"));
        fs::remove_dir_all(root).unwrap();
    }
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
        let draft = "---\ntags: [test]\n---\n# 草稿 😀\r\n[[draft-target#标题|别名]] ![[附件/中 文.png]]\r\n`[[not-a-link]]`\n";
        let report = create(
            &vault,
            Path::new("目录/a.md"),
            BTreeMap::from([
                ("目录/a.md".into(), draft.into()),
                (
                    "b.md".into(),
                    "---\naliases: [draft-target]\n---\n# 标题\n[[目录/a]]\n".into(),
                ),
            ]),
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
            BTreeMap::from([
                ("目录/a.md".into(), draft.into()),
                (
                    "b.md".into(),
                    "---\naliases: [draft-target]\n---\n# 标题\n[[目录/a]]\n".into(),
                ),
            ]),
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
