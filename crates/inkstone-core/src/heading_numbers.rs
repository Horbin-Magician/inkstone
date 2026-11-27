//! Presentation-only section numbers shared by the editor and outline.
use crate::index::Heading;

/// Use the shallowest heading as the root. Skipped ancestor levels start at one.
pub fn prefixes(headings: &[Heading]) -> Vec<String> {
    let base = headings
        .iter()
        .map(|h| h.level.clamp(1, 6))
        .min()
        .unwrap_or(1);
    let mut counters = [0usize; 6];
    headings
        .iter()
        .map(|heading| {
            let level = usize::from(heading.level.clamp(1, 6) - base);
            for counter in &mut counters[..level] {
                *counter = (*counter).max(1);
            }
            counters[level] += 1;
            counters[level + 1..].fill(0);
            counters[..=level]
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(".")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hierarchy_resets_children_and_handles_missing_levels() {
        for (source, expected) in [
            (
                "# A\n## B\n### C\n## D\n# E\n## F",
                vec!["1", "1.1", "1.1.1", "1.2", "2", "2.1"],
            ),
            ("## A\n### B\n## C", vec!["1", "1.1", "2"]),
            (
                "# A\n### B\n## C\n###### D",
                vec!["1", "1.1.1", "1.2", "1.2.1.1.1.1"],
            ),
            ("text\n```md\n# code\n```", vec![]),
        ] {
            assert_eq!(prefixes(&crate::index::parse(source).headings), expected);
        }
    }
}
