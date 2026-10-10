//! File explorer ordering, independent of filesystem access and rendering.
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, time::SystemTime};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortBy {
    #[default]
    Name,
    Modified,
    Created,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileTimes {
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
}

impl From<&std::fs::Metadata> for FileTimes {
    fn from(metadata: &std::fs::Metadata) -> Self {
        Self {
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
        }
    }
}

pub fn natural_name(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        return natural_ascii_name(a.as_bytes(), b.as_bytes());
    }
    natural_unicode_name(a, b)
}

// ASCII case folding is one byte to one byte, so compare in place instead of
// allocating two lowercase strings for every comparison in a sort.
fn natural_ascii_name(a: &[u8], b: &[u8]) -> Ordering {
    let (mut left, mut right) = (a, b);
    while let (Some(&x), Some(&y)) = (left.first(), right.first()) {
        if x.is_ascii_digit() && y.is_ascii_digit() {
            let x_len = left.iter().take_while(|c| c.is_ascii_digit()).count();
            let y_len = right.iter().take_while(|c| c.is_ascii_digit()).count();
            let x_start = left[..x_len].iter().take_while(|c| **c == b'0').count();
            let y_start = right[..y_len].iter().take_while(|c| **c == b'0').count();
            let x_digits = &left[x_start..x_len];
            let y_digits = &right[y_start..y_len];
            let order = x_digits
                .len()
                .cmp(&y_digits.len())
                .then_with(|| x_digits.cmp(y_digits));
            if order != Ordering::Equal {
                return order;
            }
            left = &left[x_len..];
            right = &right[y_len..];
        } else {
            let order = x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase());
            if order != Ordering::Equal {
                return order;
            }
            left = &left[1..];
            right = &right[1..];
        }
    }
    left.len()
        .cmp(&right.len())
        .then_with(|| {
            a.iter()
                .map(u8::to_ascii_lowercase)
                .cmp(b.iter().map(u8::to_ascii_lowercase))
        })
        .then_with(|| a.cmp(b))
}

fn natural_unicode_name(a: &str, b: &str) -> Ordering {
    let left = a.to_lowercase();
    let right = b.to_lowercase();
    let (mut a_rest, mut b_rest) = (left.as_str(), right.as_str());
    while !a_rest.is_empty() && !b_rest.is_empty() {
        let (a_char, b_char) = (
            a_rest.chars().next().unwrap(),
            b_rest.chars().next().unwrap(),
        );
        if a_char.is_ascii_digit() && b_char.is_ascii_digit() {
            let a_len = a_rest.bytes().take_while(u8::is_ascii_digit).count();
            let b_len = b_rest.bytes().take_while(u8::is_ascii_digit).count();
            let a_digits = a_rest[..a_len].trim_start_matches('0');
            let b_digits = b_rest[..b_len].trim_start_matches('0');
            let order = a_digits
                .len()
                .cmp(&b_digits.len())
                .then_with(|| a_digits.cmp(b_digits));
            if order != Ordering::Equal {
                return order;
            }
            a_rest = &a_rest[a_len..];
            b_rest = &b_rest[b_len..];
        } else {
            let order = a_char.cmp(&b_char);
            if order != Ordering::Equal {
                return order;
            }
            a_rest = &a_rest[a_char.len_utf8()..];
            b_rest = &b_rest[b_char.len_utf8()..];
        }
    }
    a_rest
        .len()
        .cmp(&b_rest.len())
        .then_with(|| left.cmp(&right))
        .then_with(|| a.cmp(b))
}

pub fn compare(
    a: (&str, bool, FileTimes),
    b: (&str, bool, FileTimes),
    by: SortBy,
    descending: bool,
) -> Ordering {
    let folders = b.1.cmp(&a.1);
    if folders != Ordering::Equal {
        return folders;
    }
    if by == SortBy::Name {
        let names = natural_name(a.0, b.0);
        return if descending { names.reverse() } else { names };
    }
    if a.1 {
        return natural_name(a.0, b.0);
    }
    let (a_time, b_time) = match by {
        SortBy::Modified => (a.2.modified, b.2.modified),
        _ => (a.2.created, b.2.created),
    };
    let times = match (a_time, b_time) {
        (Some(a), Some(b)) => {
            if descending {
                b.cmp(&a)
            } else {
                a.cmp(&b)
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        _ => Ordering::Equal,
    };
    times.then_with(|| natural_name(a.0, b.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ascii_fast_path_preserves_natural_order_and_tie_breaks() {
        let mut names = vec![String::new()];
        for prefix in ["", "a", "A", "folder/Note", "FOLDER/note", "z-"] {
            for number in [
                "",
                "0",
                "00",
                "1",
                "01",
                "2",
                "10",
                "999999999999999999999999",
            ] {
                for suffix in ["", ".md", "a2", "A02"] {
                    names.push(format!("{prefix}{number}{suffix}"));
                }
            }
        }
        for a in &names {
            for b in &names {
                assert_eq!(
                    natural_name(a, b),
                    natural_unicode_name(a, b),
                    "{a:?} vs {b:?}"
                );
            }
        }
        for a in ["中文2", "中文10", "İ2", "i\u{307}2", "ΟΣ", "ος", "😀01"] {
            for b in &names {
                assert_eq!(natural_name(a, b), natural_unicode_name(a, b));
                assert_eq!(natural_name(b, a), natural_unicode_name(b, a));
            }
        }
    }
    #[test]
    fn sorts_numbers_without_overflow_and_keeps_folders_first() {
        let mut names = [
            "笔记10",
            "笔记2",
            "笔记1",
            "笔记999999999999999999999999999999999999",
        ];
        names.sort_by(|a, b| natural_name(a, b));
        assert_eq!(names[..3], ["笔记1", "笔记2", "笔记10"]);
        assert_eq!(natural_name("a2", "A10"), Ordering::Less);
        assert_eq!(
            compare(
                ("z", true, FileTimes::default()),
                ("a", false, FileTimes::default()),
                SortBy::Name,
                true
            ),
            Ordering::Less
        );
    }
    #[test]
    fn dates_respect_direction_and_missing_values_stay_last() {
        let old = FileTimes {
            modified: Some(SystemTime::UNIX_EPOCH),
            created: None,
        };
        let new = FileTimes {
            modified: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(2)),
            created: None,
        };
        assert_eq!(
            compare(("a", false, old), ("b", false, new), SortBy::Modified, true),
            Ordering::Greater
        );
        assert_eq!(
            compare(
                ("a", false, old),
                ("b", false, new),
                SortBy::Modified,
                false
            ),
            Ordering::Less
        );
        assert_eq!(
            compare(
                ("a", false, FileTimes::default()),
                ("b", false, old),
                SortBy::Modified,
                false
            ),
            Ordering::Greater
        );
        assert_eq!(
            compare(("a", false, old), ("b", false, new), SortBy::Created, true),
            Ordering::Less
        );
    }
}
