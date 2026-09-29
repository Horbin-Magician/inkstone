//! Daily-note paths and template expansion using common Moment date tokens.
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub folder: String,
    pub format: String,
    pub template: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            folder: String::new(),
            format: "YYYY-MM-DD".into(),
            template: String::new(),
        }
    }
}
pub fn format_date(now: &DateTime<Local>, format: &str) -> Result<String, String> {
    let mut rest = format;
    let mut result = String::new();
    while !rest.is_empty() {
        if let Some(literal) = rest.strip_prefix('[') {
            let Some(end) = literal.find(']') else {
                return Err("日期格式的文字括号未闭合。".into());
            };
            result.push_str(&literal[..end]);
            rest = &literal[end + 1..];
            continue;
        }
        let tokens = [
            ("YYYY", "%Y"),
            ("YY", "%y"),
            ("MMMM", "%B"),
            ("MMM", "%b"),
            ("MM", "%m"),
            ("M", "%-m"),
            ("DD", "%d"),
            ("D", "%-d"),
            ("HH", "%H"),
            ("H", "%-H"),
            ("hh", "%I"),
            ("h", "%-I"),
            ("mm", "%M"),
            ("m", "%-M"),
            ("ss", "%S"),
            ("s", "%-S"),
            ("A", "%p"),
            ("a", "%P"),
        ];
        if let Some((token, replacement)) = tokens.iter().find(|(token, _)| rest.starts_with(token))
        {
            result.push_str(&now.format(replacement).to_string());
            rest = &rest[token.len()..];
        } else {
            let c = rest.chars().next().unwrap();
            if c.is_ascii_alphabetic() {
                return Err(format!("暂不支持日期格式符 {c}；文字请放在方括号内。"));
            }
            result.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    Ok(result)
}
fn relative(value: &str) -> Result<PathBuf, String> {
    let normalized = value.trim().replace('\\', "/");
    let path = PathBuf::from(&normalized);
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
        || normalized.contains([':', '*', '?', '"', '<', '>', '|', '\n', '\r'])
        || normalized.split('/').any(|part| part.ends_with(['.', ' ']))
    {
        return Err("请使用库内相对路径，不能包含 .. 或文件名无效字符。".into());
    }
    Ok(path)
}
impl Settings {
    pub fn path(&self, now: &DateTime<Local>, default_folder: &Path) -> Result<PathBuf, String> {
        let name = format_date(
            now,
            if self.format.trim().is_empty() {
                "YYYY-MM-DD"
            } else {
                self.format.trim()
            },
        )?;
        if name.trim().is_empty() {
            return Err("日记文件名不能为空。".into());
        }
        let folder = if self.folder.trim().is_empty() {
            default_folder.to_path_buf()
        } else {
            relative(&self.folder)?
        };
        Ok(folder.join(relative(&format!("{}.md", name.trim()))?))
    }
    pub fn template_path(&self) -> Result<Option<PathBuf>, String> {
        if self.template.trim().is_empty() {
            return Ok(None);
        }
        let mut path = relative(&self.template)?;
        if path.extension().is_none() {
            path.set_extension("md");
        }
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        {
            return Err("日记模板必须是 Markdown 文件。".into());
        }
        Ok(Some(path))
    }
}
pub fn expand_template(source: &str, title: &str, now: &DateTime<Local>) -> Result<String, String> {
    let pattern = regex::Regex::new(r"\{\{(title|date|time)(?::([^}]*))?\}\}").unwrap();
    let mut result = String::new();
    let mut end = 0;
    for captures in pattern.captures_iter(source) {
        let whole = captures.get(0).unwrap();
        result.push_str(&source[end..whole.start()]);
        if &captures[1] == "title" {
            result.push_str(title);
        } else {
            result.push_str(&format_date(
                now,
                captures
                    .get(2)
                    .map(|m| m.as_str())
                    .unwrap_or(if &captures[1] == "date" {
                        "YYYY-MM-DD"
                    } else {
                        "HH:mm"
                    }),
            )?);
        }
        end = whole.end();
    }
    result.push_str(&source[end..]);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    #[test]
    fn daily_formats_templates_and_paths() {
        let now = Local.with_ymd_and_hms(2026, 9, 29, 15, 4, 5).unwrap();
        let mut settings = Settings::default();
        assert_eq!(
            settings.path(&now, Path::new("收件箱")).unwrap(),
            Path::new("收件箱/2026-09-29.md")
        );
        settings.folder = "日记".into();
        settings.format = "YYYY/MM/YYYY-MM-DD".into();
        settings.template = "模板/每日".into();
        assert_eq!(
            settings.path(&now, Path::new("")).unwrap(),
            Path::new("日记/2026/09/2026-09-29.md")
        );
        assert_eq!(
            settings.template_path().unwrap().unwrap(),
            Path::new("模板/每日.md")
        );
        assert_eq!(
            expand_template(
                "# {{title}}\n{{date:YYYY年M月D日}} {{time}} {{time:HH:mm:ss}} {{other}}",
                "今天",
                &now
            )
            .unwrap(),
            "# 今天\n2026年9月29日 15:04 15:04:05 {{other}}"
        );
        assert_eq!(
            format_date(&now, "[Daily] YYYY.MM.DD").unwrap(),
            "Daily 2026.09.29"
        );
        for invalid in ["../外部", "C:/外部", "/外部"] {
            settings.folder = invalid.into();
            assert!(settings.path(&now, Path::new("")).is_err());
        }
        settings.template = "../outside".into();
        assert!(settings.template_path().is_err());
        assert!(format_date(&now, "[unfinished").is_err());
    }
}
