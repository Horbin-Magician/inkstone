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
    Ok(now.format(&chrono_format(format)?).to_string())
}
fn chrono_format(format: &str) -> Result<String, String> {
    let mut rest = format;
    let mut result = String::new();
    while !rest.is_empty() {
        if let Some(literal) = rest.strip_prefix('[') {
            let Some(end) = literal.find(']') else {
                return Err("日期格式的文字括号未闭合。".into());
            };
            result.push_str(&literal[..end].replace('%', "%%"));
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
            result.push_str(replacement);
            rest = &rest[token.len()..];
        } else {
            let c = rest.chars().next().unwrap();
            if c.is_ascii_alphabetic() {
                return Err(format!("暂不支持日期格式符 {c}；文字请放在方括号内。"));
            }
            if c == '%' {
                result.push('%');
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
    pub fn neighboring_note<'a>(
        &self,
        current: &Path,
        files: impl IntoIterator<Item = &'a PathBuf>,
        next: bool,
    ) -> Result<Option<PathBuf>, String> {
        let folder = relative(&self.folder)?;
        let format = chrono_format(if self.format.trim().is_empty() {
            "YYYY-MM-DD"
        } else {
            self.format.trim()
        })?;
        let parse = |path: &Path| -> Option<chrono::NaiveDateTime> {
            if !path.extension()?.eq_ignore_ascii_case("md") {
                return None;
            }
            let path = PathBuf::from(path.to_string_lossy().replace('\\', "/"));
            let parts: Vec<_> = path.components().collect();
            let prefix: Vec<_> = folder.components().collect();
            if parts.len() <= prefix.len()
                || !parts.iter().zip(&prefix).all(|(a, b)| {
                    a.as_os_str().to_string_lossy().to_lowercase()
                        == b.as_os_str().to_string_lossy().to_lowercase()
                })
            {
                return None;
            }
            let name: PathBuf = parts.into_iter().skip(prefix.len()).collect();
            let name = name.with_extension("").to_string_lossy().replace('\\', "/");
            let date = chrono::NaiveDateTime::parse_from_str(&name, &format)
                .ok()
                .or_else(|| {
                    chrono::NaiveDate::parse_from_str(&name, &format)
                        .ok()?
                        .and_hms_opt(0, 0, 0)
                })?;
            (date.format(&format).to_string() == name).then_some(date)
        };
        let current_date =
            parse(current).ok_or_else(|| "当前笔记不符合日记目录或日期格式。".to_string())?;
        let mut nearest: Option<(chrono::NaiveDateTime, PathBuf)> = None;
        for path in files {
            let Some(date) = parse(path) else {
                continue;
            };
            if (next && date > current_date || !next && date < current_date)
                && nearest
                    .as_ref()
                    .is_none_or(|(best, _)| if next { date < *best } else { date > *best })
            {
                nearest = Some((date, path.clone()));
            }
        }
        Ok(nearest.map(|(_, path)| path))
    }
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
    expand_template_with_formats(source, title, now, "YYYY-MM-DD", "HH:mm")
}
pub fn expand_template_with_formats(
    source: &str,
    title: &str,
    now: &DateTime<Local>,
    date_format: &str,
    time_format: &str,
) -> Result<String, String> {
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
                        if date_format.trim().is_empty() {
                            "YYYY-MM-DD"
                        } else {
                            date_format
                        }
                    } else {
                        if time_format.trim().is_empty() {
                            "HH:mm"
                        } else {
                            time_format
                        }
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
    fn neighboring_diaries_follow_dates_and_ignore_invalid_names() {
        let mut settings = Settings {
            folder: "日记".into(),
            format: "YYYY/MM/YYYY-MM-DD".into(),
            ..Default::default()
        };
        let files = [
            "日记/2024/03/2024-03-03.md",
            "日记/2024/02/2024-02-29.md",
            "日记/2024/03/2024-03-01.md",
            "日记/2024/02/2024-02-30.md",
            "日记副本/2024/02/2024-02-28.md",
            "日记/2024/03/2024-3-02.md",
            "日记/2024/03/2024-03-02.txt",
            "日记/2023/02/2023-02-29.md",
        ]
        .map(PathBuf::from);
        assert_eq!(
            settings.neighboring_note(&files[2], &files, false).unwrap(),
            Some(files[1].clone())
        );
        assert_eq!(
            settings.neighboring_note(&files[2], &files, true).unwrap(),
            Some(files[0].clone())
        );
        assert_eq!(
            settings.neighboring_note(&files[1], &files, false).unwrap(),
            None
        );
        assert_eq!(
            settings.neighboring_note(&files[0], &files, true).unwrap(),
            None
        );
        assert!(settings.neighboring_note(&files[4], &files, true).is_err());
        settings.folder.clear();
        settings.format = "D-M-YYYY".into();
        let files = ["31-1-2026.md", "2-2-2026.md", "10-2-2026.md"].map(PathBuf::from);
        assert_eq!(
            settings.neighboring_note(&files[1], &files, false).unwrap(),
            Some(files[0].clone())
        );
        assert_eq!(
            settings.neighboring_note(&files[1], &files, true).unwrap(),
            Some(files[2].clone())
        );
    }
    #[test]
    fn neighboring_diaries_support_time_and_escaped_literals() {
        let settings = Settings {
            format: "[100%] YYYY.MM.DD HH-mm".into(),
            ..Default::default()
        };
        let now = Local.with_ymd_and_hms(2026, 9, 29, 14, 30, 0).unwrap();
        assert_eq!(
            format_date(&now, &settings.format).unwrap(),
            "100% 2026.09.29 14-30"
        );
        let files = [
            "100% 2026.09.29 13-00.md",
            "100% 2026.09.29 14-30.md",
            "100% 2026.09.29 15-00.md",
            "100% 2026.09.29 25-00.md",
        ]
        .map(PathBuf::from);
        assert_eq!(
            settings.neighboring_note(&files[1], &files, true).unwrap(),
            Some(files[2].clone())
        );
        assert_eq!(
            settings.neighboring_note(&files[1], &files, false).unwrap(),
            Some(files[0].clone())
        );
    }
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
