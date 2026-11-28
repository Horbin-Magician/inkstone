//! Recognize a paper locally and resolve an unambiguous DOI to its deposited citation.
use pdfium_render::prelude::{PdfDocument, PdfDocumentMetadataTagType as Tag};
use regex::Regex;
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Read, sync::LazyLock, time::Duration};

#[derive(Clone, Default)]
pub(in crate::workspace) struct Paper(pub BTreeMap<&'static str, Value>);

impl Paper {
    pub(super) fn read(doc: &PdfDocument<'_>) -> Self {
        let mut fields = BTreeMap::new();
        for (tag, key) in [
            (Tag::Title, "title"),
            (Tag::Author, "authors"),
            (Tag::Subject, "subject"),
            (Tag::Keywords, "keywords"),
        ] {
            if let Some(tag) = doc.metadata().get(tag) {
                let text = tag.value().trim();
                if !text.is_empty() {
                    let value = match key {
                        "authors" => json!(parts(text, &[';'])),
                        "keywords" => json!(parts(text, &[',', ';'])),
                        _ => json!(text),
                    };
                    fields.insert(key, value);
                }
            }
        }
        let first_page = doc
            .pages()
            .get(0)
            .ok()
            .and_then(|p| p.text().ok().map(|t| t.all()))
            .unwrap_or_default();
        let metadata = fields
            .values()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        if let Some(doi) = unique_doi(&metadata).or_else(|| unique_doi(&first_page)) {
            fields.insert("doi", json!(doi));
        }
        fields.insert("page_count", json!(doc.pages().len()));
        Self(fields)
    }

    pub(in crate::workspace) fn source(&self) -> String {
        let mut source = String::from("---\n");
        for (key, value) in &self.0 {
            source.push_str(&format!("{key}: {value}\n"));
        }
        source.push_str("---\n");
        source
    }

    pub(in crate::workspace) fn lookup(mut self, doi: &str) -> anyhow::Result<Self> {
        const LIMIT: u64 = 2 * 1024 * 1024;
        let mut url = url::Url::parse("https://api.crossref.org/works/")?;
        url.path_segments_mut().unwrap().pop_if_empty().push(doi);
        let response = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(8))
            .user_agent(concat!("Inkstone/", env!("CARGO_PKG_VERSION")))
            .build()?
            .get(url)
            .send()?
            .error_for_status()?;
        let mut bytes = Vec::new();
        response.take(LIMIT + 1).read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() as u64 <= LIMIT, "Crossref 响应过大");
        let record: Value = serde_json::from_slice(&bytes)?;
        self.apply_record(doi, &record["message"])?;
        Ok(self)
    }

    fn apply_record(&mut self, doi: &str, record: &Value) -> anyhow::Result<()> {
        anyhow::ensure!(
            record["DOI"]
                .as_str()
                .is_some_and(|d| d.eq_ignore_ascii_case(doi)),
            "DOI 不匹配"
        );
        for (key, pointer) in [
            ("title", "/title/0"),
            ("publication", "/container-title/0"),
            ("publisher", "/publisher"),
            ("volume", "/volume"),
            ("issue", "/issue"),
            ("pages", "/page"),
            ("article_number", "/article-number"),
            ("item_type", "/type"),
            ("language", "/language"),
            ("url", "/URL"),
        ] {
            if let Some(text) = record
                .pointer(pointer)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                self.0.insert(key, json!(text));
            }
        }
        for (field, key) in [("ISSN", "issn"), ("ISBN", "isbn")] {
            if let Some(values) = record[field].as_array() {
                let values: Vec<_> = values.iter().filter_map(Value::as_str).collect();
                if !values.is_empty() {
                    self.0.insert(key, json!(values));
                }
            }
        }
        if let Some(markup) = record["abstract"].as_str() {
            let xml = format!(r#"<root xmlns:jats="http://jats.nlm.nih.gov">{markup}</root>"#);
            if let Ok(doc) = roxmltree::Document::parse(&xml) {
                let text = doc
                    .descendants()
                    .filter(|n| n.is_text())
                    .filter_map(|n| n.text())
                    .collect::<Vec<_>>()
                    .join(" ");
                let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !text.is_empty() {
                    self.0.insert("abstract", json!(text));
                }
            }
        }
        if let Some(authors) = record["author"].as_array() {
            let names: Vec<_> = authors
                .iter()
                .map(|a| {
                    a["name"].as_str().map(str::to_owned).unwrap_or_else(|| {
                        ["given", "family"]
                            .iter()
                            .filter_map(|k| a[k].as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                })
                .filter(|s| !s.trim().is_empty())
                .collect();
            if !names.is_empty() {
                self.0.insert("authors", json!(names));
            }
        }
        if let Some(date) = record
            .pointer("/published/date-parts/0")
            .or_else(|| record.pointer("/issued/date-parts/0"))
            .and_then(Value::as_array)
        {
            let parts: Vec<_> = date.iter().take(3).filter_map(Value::as_u64).collect();
            if let Some(year) = parts.first() {
                let mut date = year.to_string();
                for part in &parts[1..] {
                    date.push_str(&format!("-{part:02}"));
                }
                self.0.insert("publication_date", json!(date));
                self.0.insert("year", json!(year));
            }
        }
        Ok(())
    }
}

fn parts<'a>(value: &'a str, separators: &[char]) -> Vec<&'a str> {
    value
        .split(separators)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

fn unique_doi(text: &str) -> Option<String> {
    static DOI: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\b10\.[0-9]{4,9}/[-._;()/:a-z0-9]+").unwrap());
    static BODY_HEADING: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?im)^\s*(references|bibliography|参考文献|(?:1[.]?\s+)?introduction|引言)\s*$",
        )
        .unwrap()
    });
    let text = BODY_HEADING.find(text).map_or(text, |m| &text[..m.start()]);
    let mut candidates = DOI.find_iter(text).filter(|m| m.len() <= 256).map(|m| {
        let mut doi = m.as_str().trim_end_matches(['.', ';', ':']);
        while doi.ends_with(')') && doi.matches(')').count() > doi.matches('(').count() {
            doi = doi.strip_suffix(')').unwrap();
        }
        doi.to_ascii_lowercase()
    });
    let first = candidates.next()?;
    candidates.all(|d| d == first).then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bibliography_rejects_ambiguous_and_reference_dois() {
        assert_eq!(
            unique_doi("(doi:10.1016/Example(2)).\nReferences\n10.1234/ref").as_deref(),
            Some("10.1016/example(2)")
        );
        assert_eq!(unique_doi("10.1234/one 10.1234/two"), None);
        assert_eq!(unique_doi("scan\nREFERENCES\r\n10.1234/other"), None);
    }
    #[test]
    fn bibliography_maps_partial_dates_authors_and_citation_fields() {
        let mut paper = Paper::default();
        let record = json!({"DOI":"10.1234/a","title":["A: B"],"author":[{"given":"A.","family":"Smith"},{"name":"Team X"}],
            "published":{"date-parts":[[2024,3]]},"container-title":["Science"],"volume":"7","issue":"1","page":"12–17","ISSN":["1234-5678"],"abstract":"<jats:p>A &amp; B</jats:p>"});
        paper.apply_record("10.1234/A", &record).unwrap();
        assert_eq!(paper.0["authors"], json!(["A. Smith", "Team X"]));
        assert_eq!(paper.0["publication_date"], "2024-03");
        assert_eq!(paper.0["pages"], "12–17");
        assert_eq!(paper.0["abstract"], "A & B");
        assert_eq!(paper.0["issn"], json!(["1234-5678"]));
        assert_eq!(paper.0["publication"], "Science");
        assert_eq!(
            inkstone_core::properties::parse(&paper.source()).len(),
            paper.0.len()
        );
        assert!(paper.apply_record("10.1234/other", &record).is_err());
    }
}
