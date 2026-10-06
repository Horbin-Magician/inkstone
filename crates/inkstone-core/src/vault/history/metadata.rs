//! Validate legacy journal bodies without retaining decoded String copies.
//! serde_json still uses scratch space for the largest decoded string; this is
//! not a constant-memory parser and does not skip validation of body contents.
use serde::{Deserialize, Deserializer, de};
use std::{fmt, path::PathBuf};

#[derive(Deserialize)]
pub(crate) struct Metadata {
    pub root: PathBuf,
    pub relative: PathBuf,
    #[serde(rename = "baseline")]
    _baseline: Option<DiscardedString>,
    #[serde(rename = "draft")]
    _draft: DiscardedString,
}

struct DiscardedString;
impl<'de> Deserialize<'de> for DiscardedString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = DiscardedString;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a journal body string")
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(DiscardedString)
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}
