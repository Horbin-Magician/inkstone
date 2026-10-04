//! Filename ranking and bounded, ordered full-text search.

use super::{Index, SearchHit, key};

impl Index {
    pub fn filenames(&self, query: &str) -> Vec<SearchHit> {
        let query = query.trim().replace('\\', "/").to_lowercase();
        let mut matches = Vec::new();
        for (path_key, path) in self.by_path.iter() {
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
                    best = Some((rank, Some(alias.clone())));
                }
            }
            if let Some((rank, display_name)) = best {
                matches.push((
                    rank,
                    key(path),
                    SearchHit {
                        path: path.clone(),
                        display_name,
                        offset: 0,
                        line: 1,
                        excerpt: String::new(),
                        highlights: vec![],
                        title_highlights: vec![],
                    },
                ));
            }
        }
        matches.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        matches
            .into_iter()
            .take(200)
            .map(|(_, _, hit)| hit)
            .collect()
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
        if limit == 0 {
            return Ok(vec![]);
        }
        fn excerpt(text: &str, at: usize) -> String {
            let skip = text[..at].chars().count().saturating_sub(40);
            text.chars().skip(skip).take(120).collect()
        }
        let query = crate::search::Query::parse_with_case(query, case_sensitive)?;
        let mut hits = vec![];
        let mut notes: Vec<_> = self
            .notes
            .iter()
            .filter(|(path, note)| query.matches(path, &note.text, &note.parsed.tags))
            .collect();
        notes.sort_by(|(a, an), (b, bn)| {
            crate::file_order::compare(
                (&a.to_string_lossy(), false, an.times),
                (&b.to_string_lossy(), false, bn.times),
                by,
                descending,
            )
        });
        for (path, note) in notes {
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
                return Ok(hits);
            }
        }
        Ok(hits)
    }
}
