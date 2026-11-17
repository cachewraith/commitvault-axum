//! Small text utilities: slugs, LIKE patterns, excerpts.

/// Lower-case ASCII slug: `"Dev Server (EU)"` → `"dev-server-eu"`.
/// Returns an empty string when the input has no ASCII letters or digits.
pub fn slugify(input: &str) -> String {
    let mut slug = String::with_capacity(input.len());
    let mut pending_dash = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(ch.to_ascii_lowercase());
            pending_dash = false;
        } else {
            pending_dash = true;
        }
    }
    slug
}

/// Slug for `name`, falling back to `fallback` plus a short random suffix when
/// the name has no sluggable characters (e.g. non-Latin scripts).
pub fn slug_or_fallback(name: &str, fallback: &str, max_len: usize) -> String {
    let mut slug = slugify(name);
    if slug.is_empty() {
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        slug = format!("{fallback}-{}", &suffix[..8]);
    }
    truncate_slug(&mut slug, max_len);
    slug
}

fn truncate_slug(slug: &mut String, max_len: usize) {
    if slug.len() > max_len {
        slug.truncate(max_len); // ASCII only, so byte truncation is safe.
        while slug.ends_with('-') {
            slug.pop();
        }
    }
}

/// Builds a `%term%` pattern with LIKE metacharacters escaped, for use with
/// `ILIKE $n` (PostgreSQL's default escape character is the backslash).
pub fn like_contains(term: &str) -> String {
    format!("%{}%", escape_like(term))
}

/// Builds a `term%` prefix pattern with LIKE metacharacters escaped.
pub fn like_prefix(term: &str) -> String {
    format!("{}%", escape_like(term))
}

fn escape_like(term: &str) -> String {
    let mut escaped = String::with_capacity(term.len() + 2);
    for ch in term.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// Collapses whitespace and cuts at a character boundary, adding an ellipsis.
pub fn excerpt(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut chars = 0;
    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        if chars + word_len + usize::from(chars > 0) > max_chars {
            if out.is_empty() {
                out.extend(word.chars().take(max_chars));
            }
            out.push('…');
            return out;
        }
        if chars > 0 {
            out.push(' ');
            chars += 1;
        }
        out.push_str(word);
        chars += word_len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_handles_punctuation_and_case() {
        assert_eq!(slugify("Dev Server (EU)"), "dev-server-eu");
        assert_eq!(slugify("  --Hello__World--  "), "hello-world");
        assert_eq!(slugify("v2.0"), "v2-0");
        assert_eq!(slugify("ប្រព័ន្ធ"), "");
    }

    #[test]
    fn slug_fallback_and_truncation() {
        let slug = slug_or_fallback("ប្រព័ន្ធ", "project", 140);
        assert!(slug.starts_with("project-") && slug.len() == "project-".len() + 8);
        assert_eq!(slug_or_fallback("aaaa-bbbb", "x", 5), "aaaa");
    }

    #[test]
    fn like_patterns_escape_metacharacters() {
        assert_eq!(like_contains("50%_off\\"), "%50\\%\\_off\\\\%");
        assert_eq!(like_prefix("ab_"), "ab\\_%");
    }

    #[test]
    fn excerpt_collapses_whitespace_and_truncates_on_words() {
        assert_eq!(excerpt("  hello \n\n world  ", 50), "hello world");
        assert_eq!(excerpt("hello wonderful world", 12), "hello…");
        assert_eq!(excerpt("supercalifragilistic", 5), "super…");
        assert_eq!(excerpt("", 5), "");
    }
}
