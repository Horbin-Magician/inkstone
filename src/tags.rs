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
}
impl Default for Options {
    fn default() -> Self {
        Self {
            hierarchy: true,
            sort: Sort::Frequency,
            collapsed: BTreeSet::new(),
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
    let mut counts: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for note in index.notes.values() {
        for tag in &note.parsed.tags {
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
                counts
                    .entry(prefix.to_lowercase())
                    .or_insert_with(|| (prefix.clone(), 0))
                    .1 += 1;
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
    for key in counts.keys() {
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

#[cfg(test)]
mod tests {
    use super::*;
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
