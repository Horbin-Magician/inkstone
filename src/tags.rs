//! Tag pane hierarchy, counts and ordering.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sort {
    Name,
    NameDescending,
    #[default]
    Frequency,
    FrequencyAscending,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub hierarchy: bool,
    pub sort: Sort,
    pub collapsed: BTreeSet<String>,
    pub show_filter: bool,
    pub query: String,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            hierarchy: true,
            sort: Sort::Frequency,
            collapsed: BTreeSet::new(),
            show_filter: false,
            query: String::new(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Row {
    pub tag: String,
    pub count: usize,
    pub depth: usize,
    pub children: bool,
    pub collapsed: bool,
}
pub fn rows(index: &crate::index::Index, options: &Options) -> Vec<Row> {
    let mut spellings: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for note in index.notes.values() {
        for (tag, count) in &note.parsed.tag_counts {
            let mut prefix = String::new();
            for part in tag
                .trim_end_matches('/')
                .split('/')
                .filter(|part| !part.is_empty())
            {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(part);
                *spellings
                    .entry(prefix.to_lowercase())
                    .or_default()
                    .entry(prefix.clone())
                    .or_default() += count;
            }
        }
    }
    let counts: BTreeMap<String, (String, usize)> = spellings
        .into_iter()
        .map(|(key, versions)| {
            let display = versions
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
                .unwrap()
                .0
                .clone();
            (key, (display, versions.values().sum()))
        })
        .collect();
    let mut visible: BTreeSet<String> = counts.keys().cloned().collect();
    if options.show_filter && !options.query.trim().is_empty() {
        let Ok(query) = crate::search::Query::parse(&options.query) else {
            return vec![];
        };
        visible.clear();
        for (key, (tag, _)) in &counts {
            if query.matches(
                std::path::Path::new(""),
                &format!("#{tag}"),
                std::slice::from_ref(tag),
            ) {
                visible.insert(key.clone());
                if options.hierarchy {
                    let mut current = key.as_str();
                    while let Some((parent, _)) = current.rsplit_once('/') {
                        visible.insert(parent.to_owned());
                        current = parent;
                    }
                }
            }
        }
    }
    let compare = |a: &String, b: &String| {
        let names = crate::file_order::natural_name(&counts[a].0, &counts[b].0);
        match options.sort {
            Sort::Name => names,
            Sort::NameDescending => names.reverse(),
            Sort::Frequency => counts[b].1.cmp(&counts[a].1).then(names),
            Sort::FrequencyAscending => counts[a].1.cmp(&counts[b].1).then(names),
        }
    };
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for key in &visible {
        let parent = if options.hierarchy {
            key.rsplit_once('/').map(|(parent, _)| parent).unwrap_or("")
        } else {
            ""
        };
        children.entry(parent.into()).or_default().push(key.clone());
    }
    for values in children.values_mut() {
        values.sort_by(compare);
    }
    let mut stack = children.get("").cloned().unwrap_or_default();
    stack.reverse();
    let mut result = vec![];
    while let Some(key) = stack.pop() {
        let nested = children.get(&key);
        let collapsed = options.collapsed.contains(&key);
        let (tag, count) = &counts[&key];
        result.push(Row {
            tag: tag.clone(),
            count: *count,
            depth: if options.hierarchy {
                key.matches('/').count()
            } else {
                0
            },
            children: nested.is_some(),
            collapsed,
        });
        if !collapsed && let Some(nested) = nested {
            stack.extend(nested.iter().rev().cloned());
        }
    }
    result
}

#[derive(Debug, PartialEq, Eq)]
pub enum Navigation {
    Select(String),
    Fold(String, bool),
    Open(String),
    None,
}
pub fn navigate(items: &[Row], selected: Option<&str>, key: &str) -> Navigation {
    let found = selected.and_then(|selected| {
        items
            .iter()
            .position(|row| row.tag.to_lowercase() == selected)
    });
    let Some(row) = items.get(found.unwrap_or(0)) else {
        return Navigation::None;
    };
    let index = found.unwrap_or(0);
    let target = match key {
        "down" => Some(found.map_or(0, |i| (i + 1).min(items.len() - 1))),
        "up" => Some(found.map_or(items.len() - 1, |i| i.saturating_sub(1))),
        "home" => Some(0),
        "end" => Some(items.len() - 1),
        "right" if row.children && row.collapsed => {
            return Navigation::Fold(row.tag.clone(), false);
        }
        "right" if row.children => Some((index + 1).min(items.len() - 1)),
        "left" if row.children && !row.collapsed => return Navigation::Fold(row.tag.clone(), true),
        "left" if row.depth > 0 => items[..index]
            .iter()
            .rposition(|parent| parent.depth < row.depth),
        "enter" => return Navigation::Open(row.tag.clone()),
        _ => None,
    };
    target
        .map(|i| Navigation::Select(items[i].tag.clone()))
        .unwrap_or(Navigation::None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_tags_count_occurrences_and_prefer_the_most_common_spelling() {
        let mut index = crate::index::Index::default();
        index.update("a.md".into(), "---\nTags: [WORK/one, WORK/one]\n---\n#work/one #work/one #work/one\n`#work/one`\n```\n#work/one\n```".into());
        let items = rows(&index, &Options::default());
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].tag, "work");
        assert_eq!(items[0].count, 5);
        assert_eq!(items[1].tag, "work/one");
        assert_eq!(items[1].count, 5);
        assert_eq!(
            index.notes[std::path::Path::new("a.md")].parsed.tags.len(),
            2
        );
    }
    #[test]
    fn keyboard_navigation_handles_tree_edges_and_stale_selection() {
        let mut index = crate::index::Index::default();
        index.update("a.md".into(), "#work/one #work/two".into());
        let options = Options::default();
        let items = rows(&index, &options);
        assert_eq!(
            navigate(&items, None, "down"),
            Navigation::Select("work".into())
        );
        assert_eq!(
            navigate(&items, Some("work/one"), "left"),
            Navigation::Select("work".into())
        );
        assert_eq!(
            navigate(&items, Some("work"), "right"),
            Navigation::Select("work/one".into())
        );
        assert_eq!(
            navigate(&items, Some("work/two"), "down"),
            Navigation::Select("work/two".into())
        );
        assert_eq!(
            navigate(&items, Some("missing"), "enter"),
            Navigation::Open("work".into())
        );
        assert_eq!(
            navigate(&items, None, "end"),
            Navigation::Select("work/two".into())
        );
        assert_eq!(navigate(&[], None, "down"), Navigation::None);
    }
    #[test]
    fn filtering_preserves_ancestors_counts_and_saved_folds() {
        let mut index = crate::index::Index::default();
        index.update("a.md".into(), "#工作/会议 #工作/项目 #私人/日记".into());
        let mut options = Options {
            show_filter: true,
            query: "会议".into(),
            ..Default::default()
        };
        let filtered = rows(&index, &options);
        assert_eq!(
            filtered
                .iter()
                .map(|row| row.tag.as_str())
                .collect::<Vec<_>>(),
            ["工作", "工作/会议"]
        );
        assert_eq!(filtered[0].count, 2);
        options.collapsed.insert("工作".into());
        assert_eq!(rows(&index, &options).len(), 1);
        options.hierarchy = false;
        assert_eq!(rows(&index, &options)[0].tag, "工作/会议");
        options.query = "/会议|日记/ -私人".into();
        assert_eq!(rows(&index, &options).len(), 1);
        options.query = "/[bad/".into();
        assert!(rows(&index, &options).is_empty());
        options.show_filter = false;
        assert_eq!(rows(&index, &options).len(), 5);
        assert!(options.collapsed.contains("工作"));
    }
    #[test]
    fn hierarchy_counts_parents_sorts_siblings_and_restores_folds() {
        let mut index = crate::index::Index::default();
        index.update("a.md".into(), "#work/one #work/two #other".into());
        index.update("b.md".into(), "#Work/one #item10 #item2".into());
        let mut options = Options::default();
        let items = rows(&index, &options);
        assert_eq!(items[0].tag.to_lowercase(), "work");
        assert_eq!(items[0].count, 3);
        assert_eq!(items[1].tag.to_lowercase(), "work/one");
        assert_eq!(items[1].count, 2);
        assert_eq!(items[1].depth, 1);
        options.collapsed.insert("work".into());
        assert!(!rows(&index, &options).iter().any(|row| row.depth > 0));
        options.hierarchy = false;
        options.sort = Sort::Name;
        let items = rows(&index, &options);
        assert_eq!(items[0].tag, "item2");
        assert_eq!(items[1].tag, "item10");
        assert!(items.iter().any(|row| row.tag.to_lowercase() == "work/one"));
    }
}
