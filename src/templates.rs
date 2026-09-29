//! Local template configuration and directory matching.
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub folder: String,
    pub date_format: String,
    pub time_format: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            folder: String::new(),
            date_format: "YYYY-MM-DD".into(),
            time_format: "HH:mm".into(),
        }
    }
}
impl Settings {
    pub fn directory(&self) -> Result<PathBuf, String> {
        let folder = self.folder.trim().replace('\\', "/");
        if folder.is_empty() {
            return Err("请先在设置中指定模板文件夹。".into());
        }
        let path = PathBuf::from(folder);
        if path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err("模板文件夹必须是库内的非根目录，不能包含 ..。".into());
        }
        Ok(path)
    }
    pub fn expand(
        &self,
        source: &str,
        title: &str,
        now: &chrono::DateTime<chrono::Local>,
    ) -> Result<String, String> {
        crate::daily::expand_template_with_formats(
            source,
            title,
            now,
            &self.date_format,
            &self.time_format,
        )
    }
}
pub fn in_folder(path: &Path, folder: &Path) -> bool {
    let path = path.to_string_lossy().replace('\\', "/").to_lowercase();
    let folder = folder
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase();
    path.strip_prefix(&folder)
        .is_some_and(|rest| rest.starts_with('/'))
}
pub fn search(
    index: &crate::index::Index,
    folder: &Path,
    query: &str,
) -> Vec<crate::index::SearchHit> {
    let query = query.trim().replace('\\', "/").to_lowercase();
    let depth = folder.components().count();
    index
        .notes
        .keys()
        .filter(|path| in_folder(path, folder))
        .filter_map(|path| {
            let relative: PathBuf = path.components().skip(depth).collect();
            let name = relative
                .with_extension("")
                .to_string_lossy()
                .replace('\\', "/");
            name.to_lowercase()
                .contains(&query)
                .then(|| crate::index::SearchHit {
                    path: path.clone(),
                    display_name: Some(name),
                    offset: 0,
                    line: 1,
                    excerpt: String::new(),
                })
        })
        .take(200)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    #[test]
    fn template_settings_respect_folder_boundaries_and_formats() {
        let mut settings = Settings::default();
        assert!(settings.directory().is_err());
        settings.folder = "模板\\子目录".into();
        assert_eq!(settings.directory().unwrap(), Path::new("模板/子目录"));
        assert!(in_folder(
            Path::new("模板/子目录/嵌套/a.md"),
            &settings.directory().unwrap()
        ));
        assert!(!in_folder(
            Path::new("模板/子目录副本/a.md"),
            &settings.directory().unwrap()
        ));
        settings.date_format = "YYYY年M月D日".into();
        settings.time_format = "HH:mm:ss".into();
        let now = chrono::Local
            .with_ymd_and_hms(2026, 9, 29, 14, 3, 5)
            .unwrap();
        assert_eq!(
            settings
                .expand("{{date}} {{time}} {{date:YYYY}} {{title}}", "笔记", &now)
                .unwrap(),
            "2026年9月29日 14:03:05 2026 笔记"
        );
        settings.folder = "../外部".into();
        assert!(settings.directory().is_err());
    }
    #[test]
    fn template_search_filters_before_limit_and_displays_relative_names() {
        let mut index = crate::index::Index::default();
        for i in 0..250 {
            index.update(format!("other/{i}.md").into(), String::new());
        }
        index.update("模板/子目录/会议.md".into(), String::new());
        index.update("模板副本/会议.md".into(), String::new());
        let hits = search(&index, Path::new("模板"), "");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].display_name.as_deref(), Some("子目录/会议"));
        assert_eq!(search(&index, Path::new("模板"), "子目录\\会").len(), 1);
        assert!(search(&index, Path::new("模板"), "missing").is_empty());
    }
}
