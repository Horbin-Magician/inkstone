//! Filename ranking and bounded, ordered full-text search.

use super::{Index, SearchHit};

impl Index {
    pub fn filenames(&self, query: &str) -> Vec<SearchHit> {
        self.filenames_cancellable(query, || false)
            .unwrap_or_default()
    }
    /// None means cancelled; never expose partial results from an obsolete query.
    pub fn filenames_cancellable(
        &self,
        query: &str,
        cancelled: impl Fn() -> bool,
    ) -> Option<Vec<SearchHit>> {
        if cancelled() {
            return None;
        }
        let query = query.trim().replace('\\', "/").to_lowercase();
        let mut matches = Vec::new();
        for (path_key, path) in self.by_path.iter() {
            if cancelled() {
                return None;
            }
            let file = path_key.rsplit('/').next().unwrap_or(path_key);
            let stem = file.strip_suffix(".md").unwrap_or(file);
            let mut best = if stem == query {
                Some((0, None))
            } else if stem.starts_with(&query) {
                Some((2, None))
            } else if path_key.contains(&query) {
                Some((4, None))
            } else {
                None
            };
            let Some(note) = self.notes.get(path) else {
                continue;
            };
            for alias in &note.parsed.aliases {
                let name = alias.to_lowercase();
                let rank = if name == query {
                    1
                } else if name.starts_with(&query) {
                    3
                } else if name.contains(&query) {
                    5
                } else {
                    continue;
                };
                if best.as_ref().is_none_or(|(old, _)| rank < *old) {
                    best = Some((rank, Some(alias)));
                }
            }
            if let Some((rank, display_name)) = best {
                matches.push((rank, path_key, path, display_name));
            }
        }
        if cancelled() {
            return None;
        }
        let compare =
            |a: &(u8, &String, &std::path::PathBuf, Option<&String>),
             b: &(u8, &String, &std::path::PathBuf, Option<&String>)| {
                a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1))
            };
        // Rank only the visible prefix and allocate owned hits for that prefix.
        if matches.len() > 200 {
            matches.select_nth_unstable_by(200, compare);
            matches.truncate(200);
        }
        matches.sort_unstable_by(compare);
        if cancelled() {
            return None;
        }
        Some(
            matches
                .into_iter()
                .map(|(_, _, path, display_name)| SearchHit {
                    path: path.clone(),
                    display_name: display_name.cloned(),
                    offset: 0,
                    line: 1,
                    excerpt: String::new(),
                    highlights: vec![],
                    title_highlights: vec![],
                })
                .collect(),
        )
    }
    #[cfg(test)]
    pub(crate) fn search(&self, query: &str) -> Vec<SearchHit> {
        self.try_search(query).unwrap_or_default()
    }
    #[cfg(test)]
    pub(super) fn try_search(&self, query: &str) -> Result<Vec<SearchHit>, String> {
        self.search_with_case(query, false)
    }
    #[cfg(test)]
    pub(super) fn search_with_case(
        &self,
        query: &str,
        case_sensitive: bool,
    ) -> Result<Vec<SearchHit>, String> {
        self.search_ordered(
            query,
            case_sensitive,
            crate::file_order::SortBy::Name,
            false,
        )
    }
    #[cfg(test)]
    pub(super) fn search_ordered(
        &self,
        query: &str,
        case_sensitive: bool,
        by: crate::file_order::SortBy,
        descending: bool,
    ) -> Result<Vec<SearchHit>, String> {
        self.search_limited(query, case_sensitive, by, descending, 200)
    }
    pub fn search_limited(
        &self,
        query: &str,
        case_sensitive: bool,
        by: crate::file_order::SortBy,
        descending: bool,
        limit: usize,
    ) -> Result<Vec<SearchHit>, String> {
        self.search_limited_cancellable(query, case_sensitive, by, descending, limit, || false)
            .map(Option::unwrap_or_default)
    }
    /// Cancellation is checked between notes and around ordering. A single note
    /// match is atomic; this does not interrupt a regex already examining it.
    pub fn search_limited_cancellable(
        &self,
        query: &str,
        case_sensitive: bool,
        by: crate::file_order::SortBy,
        descending: bool,
        limit: usize,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Vec<SearchHit>>, String> {
        if cancelled() {
            return Ok(None);
        }
        if limit == 0 {
            return Ok(Some(vec![]));
        }
        fn excerpt(text: &str, at: usize) -> String {
            let skip = text[..at].chars().count().saturating_sub(40);
            text.chars().skip(skip).take(120).collect()
        }
        let query = crate::search::Query::parse_with_case(query, case_sensitive)?;
        let mut hits = vec![];
        // Before reaching the cap, discard nonmatches without sorting them.
        // Once enough documents match, defer the rest of the matching until
        // after sorting so common queries can stop early. Every matching note
        // produces at least one hit, including metadata-only matches.
        let mut confirmed = 0;
        let mut notes = Vec::new();
        for (path, note) in &self.notes {
            if cancelled() {
                return Ok(None);
            }
            if confirmed == limit {
                notes.push((path, note, false));
            } else if query.matches(path, &note.text, &note.parsed.tags) {
                confirmed += 1;
                notes.push((path, note, true));
            }
        }
        if cancelled() {
            return Ok(None);
        }
        notes.sort_by(|(a, an, _), (b, bn, _)| {
            crate::file_order::compare(
                (&a.to_string_lossy(), false, an.times),
                (&b.to_string_lossy(), false, bn.times),
                by,
                descending,
            )
        });
        for (path, note, matched) in notes {
            if cancelled() {
                return Ok(None);
            }
            if !matched && !query.matches(path, &note.text, &note.parsed.tags) {
                continue;
            }
            let title_highlights = query.title_highlights(path, &note.text, &note.parsed.tags);
            let before = hits.len();
            for found in
                query.matching_lines(path, &note.text, &note.parsed.tags, limit - hits.len())
            {
                hits.push(SearchHit {
                    path: path.clone(),
                    display_name: None,
                    offset: found.offset,
                    line: found.line,
                    excerpt: note.text[found.range].to_string(),
                    highlights: found.highlights,
                    title_highlights: title_highlights.clone(),
                });
            }
            if hits.len() == before {
                let at = query
                    .first_offset(path, &note.text, &note.parsed.tags)
                    .unwrap_or(0);
                let start = note.text[..at].rfind('\n').map_or(0, |i| i + 1);
                let end = note.text[at..]
                    .find('\n')
                    .map_or(note.text.len(), |i| at + i);
                hits.push(SearchHit {
                    path: path.clone(),
                    display_name: None,
                    offset: at,
                    line: note.text[..at]
                        .bytes()
                        .filter(|byte| *byte == b'\n')
                        .count()
                        + 1,
                    excerpt: excerpt(&note.text[start..end], at - start),
                    highlights: vec![],
                    title_highlights,
                });
            }
            if hits.len() == limit {
                return Ok((!cancelled()).then_some(hits));
            }
        }
        Ok((!cancelled()).then_some(hits))
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use crate::file_order::SortBy;
    use std::cell::Cell;

    #[test]
    fn obsolete_queries_stop_scanning_without_partial_hits() {
        let mut index = Index::default();
        for i in 0..32 {
            index.update(
                format!("{i:02}.md").into(),
                "中文😀 first\n中文 second".into(),
            );
        }
        for stop_after in [8, 35] {
            let checked = Cell::new(0);
            let hits = index
                .search_limited_cancellable("中文", false, SortBy::Name, false, 3, || {
                    checked.set(checked.get() + 1);
                    checked.get() >= stop_after
                })
                .unwrap();
            assert!(
                hits.is_none(),
                "cancelled scans cannot publish partial hits"
            );
            assert_eq!(checked.get(), stop_after);
        }
        let checked = Cell::new(0);
        assert!(
            index
                .filenames_cancellable("md", || {
                    checked.set(checked.get() + 1);
                    checked.get() == 8
                })
                .is_none()
        );
        assert_eq!(
            checked.get(),
            8,
            "filename scan must stop before all 32 notes"
        );
        let hits = index
            .search_limited_cancellable("中文", false, SortBy::Name, false, 3, || false)
            .unwrap()
            .unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].path.to_str(), Some("00.md"));
        assert_eq!(hits[0].line, 1);
        assert_eq!(hits[1].line, 2);
        assert_eq!(hits[1].offset, "中文😀 first\n".len());
        assert_eq!(hits[2].path.to_str(), Some("01.md"));
        assert!(
            index
                .search_limited_cancellable("/[/", false, SortBy::Name, false, 3, || false)
                .is_err()
        );
    }
}
