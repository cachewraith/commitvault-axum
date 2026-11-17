//! Deserialisation helpers that normalise user text before validation runs.

use serde::{Deserialize, Deserializer};

/// Trims surrounding whitespace.
pub fn trimmed<'de, D: Deserializer<'de>>(de: D) -> Result<String, D::Error> {
    Ok(String::deserialize(de)?.trim().to_owned())
}

/// Trims; blank or `null` becomes `None`.
pub fn trimmed_opt<'de, D: Deserializer<'de>>(de: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(de)?.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()))
}

/// Trims and lower-cases; blank or `null` becomes `None` (hex colours, hashes).
pub fn lowercase_opt<'de, D: Deserializer<'de>>(de: D) -> Result<Option<String>, D::Error> {
    Ok(trimmed_opt(de)?.map(|s| s.to_lowercase()))
}

/// Trims and lower-cases.
pub fn lowercase<'de, D: Deserializer<'de>>(de: D) -> Result<String, D::Error> {
    Ok(trimmed(de)?.to_lowercase())
}

/// PATCH semantics for nullable fields: absent → `None` (leave unchanged),
/// `null` → `Some(None)` (clear), value → `Some(Some(v))`.
/// Use together with `#[serde(default)]`.
pub fn patch<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

/// `patch` for text: trims, and treats blank like `null` (clear).
pub fn patch_text<'de, D: Deserializer<'de>>(de: D) -> Result<Option<Option<String>>, D::Error> {
    trimmed_opt(de).map(Some)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct Sample {
        #[serde(deserialize_with = "super::trimmed")]
        name: String,
        #[serde(default, deserialize_with = "super::trimmed_opt")]
        note: Option<String>,
        #[serde(default, deserialize_with = "super::patch_text")]
        description: Option<Option<String>>,
        #[serde(default, deserialize_with = "super::patch")]
        count: Option<Option<u32>>,
    }

    fn parse(json: &str) -> Sample {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn trims_and_blanks_become_none() {
        let s = parse(r#"{"name":"  a  ","note":"   "}"#);
        assert_eq!(s.name, "a");
        assert_eq!(s.note, None);
    }

    #[test]
    fn patch_distinguishes_absent_null_and_value() {
        let absent = parse(r#"{"name":"a"}"#);
        assert_eq!(absent.description, None);
        assert_eq!(absent.count, None);

        let null = parse(r#"{"name":"a","description":null,"count":null}"#);
        assert_eq!(null.description, Some(None));
        assert_eq!(null.count, Some(None));

        let value = parse(r#"{"name":"a","description":" x ","count":3}"#);
        assert_eq!(value.description, Some(Some("x".into())));
        assert_eq!(value.count, Some(Some(3)));

        let blank = parse(r#"{"name":"a","description":"  "}"#);
        assert_eq!(blank.description, Some(None));
    }
}
