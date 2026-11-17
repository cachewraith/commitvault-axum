//! Server-side validation of Tiptap (ProseMirror) JSON documents.
//!
//! Rich text is stored as structured JSON, never as HTML. The frontend editor
//! only produces the node and mark types listed here, but the API must not
//! trust that: every document is rebuilt from an allow-list before it reaches
//! the database, so stored content can only ever contain known node types,
//! known attributes with validated values, and safe link/image URLs.
//!
//! Policy:
//! * unknown node or mark type  → the document is rejected;
//! * unknown attribute          → dropped;
//! * link with an unsafe `href` → the mark is dropped, the text is kept;
//! * image with an unsafe `src` → the node is dropped.

use serde_json::{Map, Value, json};

use super::validation::is_safe_http_url;

const MAX_DEPTH: usize = 40;
const MAX_NODES: usize = 50_000;
const MAX_ATTR_TEXT: usize = 500;
const MAX_HEADING_LEVEL: u64 = 4;

/// A document that passed validation, plus its plain-text projection (used for
/// search and excerpts).
#[derive(Debug, Clone, PartialEq)]
pub struct SanitizedDocument {
    pub json: Value,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct RichTextError(String);

fn invalid<T>(message: impl Into<String>) -> Result<T, RichTextError> {
    Err(RichTextError(message.into()))
}

pub fn empty_document() -> Value {
    json!({ "type": "doc", "content": [{ "type": "paragraph" }] })
}

pub fn sanitize_document(input: &Value) -> Result<SanitizedDocument, RichTextError> {
    if input.get("type").and_then(Value::as_str) != Some("doc") {
        return invalid("Document root must be a node of type `doc`");
    }
    let mut ctx = Context { nodes: 0, text: String::new() };
    match ctx.node(input, 0)? {
        Some(json) => Ok(SanitizedDocument { json, text: ctx.text.trim().to_owned() }),
        None => invalid("Document is empty"),
    }
}

struct Context {
    nodes: usize,
    text: String,
}

impl Context {
    /// Returns the sanitised node, or `None` when the node must be dropped.
    fn node(&mut self, value: &Value, depth: usize) -> Result<Option<Value>, RichTextError> {
        if depth > MAX_DEPTH {
            return invalid("Document is nested too deeply");
        }
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return invalid("Document is too large");
        }
        let Some(object) = value.as_object() else {
            return invalid("Document nodes must be objects");
        };
        let Some(kind) = object.get("type").and_then(Value::as_str) else {
            return invalid("Document node is missing its `type`");
        };
        let attrs = object.get("attrs").and_then(Value::as_object);

        let mut out = Map::new();
        out.insert("type".into(), Value::String(kind.to_owned()));

        match kind {
            "text" => return self.text_node(object, out),
            "doc" if depth == 0 => {}
            "paragraph" | "bulletList" | "listItem" | "taskList" | "blockquote" | "table" | "tableRow"
            | "horizontalRule" => {}
            "hardBreak" => self.text.push('\n'),
            "heading" => {
                let level = attrs.and_then(|a| a.get("level")).and_then(Value::as_u64).unwrap_or(1);
                if !(1..=MAX_HEADING_LEVEL).contains(&level) {
                    return invalid(format!("Heading level must be between 1 and {MAX_HEADING_LEVEL}"));
                }
                out.insert("attrs".into(), json!({ "level": level }));
            }
            "orderedList" => {
                let start = attrs.and_then(|a| a.get("start")).and_then(Value::as_u64).unwrap_or(1).max(1);
                // HTML list numbering style; anything else falls back to the default.
                let numbering = attrs
                    .and_then(|a| a.get("type"))
                    .and_then(Value::as_str)
                    .filter(|t| matches!(*t, "1" | "a" | "A" | "i" | "I"))
                    .map_or(Value::Null, |t| Value::String(t.to_owned()));
                out.insert("attrs".into(), json!({ "start": start, "type": numbering }));
            }
            "taskItem" => {
                let checked = attrs.and_then(|a| a.get("checked")).and_then(Value::as_bool).unwrap_or(false);
                out.insert("attrs".into(), json!({ "checked": checked }));
            }
            "codeBlock" => {
                let language = attrs
                    .and_then(|a| a.get("language"))
                    .and_then(Value::as_str)
                    .filter(|l| is_language_name(l))
                    .map_or(Value::Null, |l| Value::String(l.to_owned()));
                out.insert("attrs".into(), json!({ "language": language }));
            }
            "tableCell" | "tableHeader" => {
                out.insert("attrs".into(), table_cell_attrs(attrs));
            }
            "image" => {
                let Some(src) =
                    attrs.and_then(|a| a.get("src")).and_then(Value::as_str).filter(|s| is_safe_http_url(s))
                else {
                    return Ok(None);
                };
                let mut image = Map::new();
                image.insert("src".into(), Value::String(src.to_owned()));
                for key in ["alt", "title"] {
                    let text = attrs.and_then(|a| a.get(key)).and_then(Value::as_str);
                    if let Some(text) = text.filter(|t| t.chars().count() <= MAX_ATTR_TEXT) {
                        image.insert(key.into(), Value::String(text.to_owned()));
                        if key == "alt" {
                            self.text.push_str(text);
                            self.text.push('\n');
                        }
                    }
                }
                out.insert("attrs".into(), Value::Object(image));
                return Ok(Some(Value::Object(out)));
            }
            other => return invalid(format!("Unsupported content type `{other}`")),
        }

        if let Some(children) = object.get("content") {
            let Some(children) = children.as_array() else {
                return invalid("Node `content` must be an array");
            };
            let mut sanitised = Vec::with_capacity(children.len());
            for child in children {
                if let Some(node) = self.node(child, depth + 1)? {
                    sanitised.push(node);
                }
            }
            if !sanitised.is_empty() {
                out.insert("content".into(), Value::Array(sanitised));
            }
        }

        if matches!(kind, "paragraph" | "heading" | "codeBlock" | "tableCell" | "tableHeader") {
            self.text.push('\n');
        }
        Ok(Some(Value::Object(out)))
    }

    fn text_node(
        &mut self,
        object: &Map<String, Value>,
        mut out: Map<String, Value>,
    ) -> Result<Option<Value>, RichTextError> {
        let Some(text) = object.get("text").and_then(Value::as_str) else {
            return invalid("Text node is missing its `text`");
        };
        if text.is_empty() {
            return Ok(None); // ProseMirror forbids empty text nodes.
        }
        self.text.push_str(text);
        out.insert("text".into(), Value::String(text.to_owned()));

        if let Some(marks) = object.get("marks") {
            let Some(marks) = marks.as_array() else {
                return invalid("Text `marks` must be an array");
            };
            let mut sanitised = Vec::with_capacity(marks.len());
            for mark in marks {
                if let Some(mark) = sanitize_mark(mark)? {
                    sanitised.push(mark);
                }
            }
            if !sanitised.is_empty() {
                out.insert("marks".into(), Value::Array(sanitised));
            }
        }
        Ok(Some(Value::Object(out)))
    }
}

fn sanitize_mark(mark: &Value) -> Result<Option<Value>, RichTextError> {
    let Some(kind) = mark.get("type").and_then(Value::as_str) else {
        return invalid("Mark is missing its `type`");
    };
    match kind {
        "bold" | "italic" | "underline" | "strike" | "code" => Ok(Some(json!({ "type": kind }))),
        "link" => {
            let href = mark.get("attrs").and_then(|a| a.get("href")).and_then(Value::as_str);
            Ok(href.filter(|h| is_safe_link(h)).map(|href| {
                json!({
                    "type": "link",
                    // target/rel are fixed server-side so stored links can never
                    // gain `window.opener` access or pass referrers.
                    "attrs": { "href": href, "target": "_blank", "rel": "noopener noreferrer nofollow" }
                })
            }))
        }
        other => invalid(format!("Unsupported text formatting `{other}`")),
    }
}

/// Links may point at http(s) resources or be `mailto:` addresses. Everything
/// else (`javascript:`, `data:`, `vbscript:`, relative URLs…) is refused.
pub fn is_safe_link(href: &str) -> bool {
    if is_safe_http_url(href) {
        return true;
    }
    href.len() <= 320
        && !href.chars().any(|c| c.is_control() || c.is_whitespace())
        && url::Url::parse(href).is_ok_and(|url| url.scheme() == "mailto" && url.path().contains('@'))
}

fn is_language_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'#' | b'-' | b'_' | b'.'))
}

fn table_cell_attrs(attrs: Option<&Map<String, Value>>) -> Value {
    let span = |key: &str| attrs.and_then(|a| a.get(key)).and_then(Value::as_u64).unwrap_or(1).clamp(1, 100);
    let colwidth = attrs
        .and_then(|a| a.get("colwidth"))
        .and_then(Value::as_array)
        .filter(|widths| widths.len() <= 100 && widths.iter().all(|w| w.as_u64().is_some_and(|w| w <= 4000)))
        .map_or(Value::Null, |widths| Value::Array(widths.clone()));
    let align = attrs
        .and_then(|a| a.get("align"))
        .and_then(Value::as_str)
        .filter(|a| matches!(*a, "left" | "center" | "right"))
        .map_or(Value::Null, |a| Value::String(a.to_owned()));
    json!({ "colspan": span("colspan"), "rowspan": span("rowspan"), "colwidth": colwidth, "align": align })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(content: Value) -> Value {
        json!({ "type": "doc", "content": content })
    }

    fn text_of(value: Value) -> String {
        sanitize_document(&value).unwrap().text
    }

    #[test]
    fn accepts_every_supported_node_and_mark() {
        let input = doc(json!([
            { "type": "heading", "attrs": { "level": 2 }, "content": [{ "type": "text", "text": "Deploy" }] },
            { "type": "paragraph", "content": [
                { "type": "text", "text": "bold", "marks": [{ "type": "bold" }, { "type": "italic" }, { "type": "underline" }, { "type": "strike" }] },
                { "type": "hardBreak" },
                { "type": "text", "text": "cargo run", "marks": [{ "type": "code" }] },
                { "type": "text", "text": "docs", "marks": [{ "type": "link", "attrs": { "href": "https://example.com/a?b=1" } }] }
            ]},
            { "type": "bulletList", "content": [{ "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] }] },
            { "type": "orderedList", "attrs": { "start": 3 }, "content": [{ "type": "listItem", "content": [{ "type": "paragraph" }] }] },
            { "type": "taskList", "content": [{ "type": "taskItem", "attrs": { "checked": true }, "content": [{ "type": "paragraph" }] }] },
            { "type": "blockquote", "content": [{ "type": "paragraph" }] },
            { "type": "codeBlock", "attrs": { "language": "rust" }, "content": [{ "type": "text", "text": "fn main() {}" }] },
            { "type": "horizontalRule" },
            { "type": "image", "attrs": { "src": "https://example.com/a.png", "alt": "diagram" } },
            { "type": "table", "content": [{ "type": "tableRow", "content": [
                { "type": "tableHeader", "attrs": { "colspan": 1, "rowspan": 1, "colwidth": null }, "content": [{ "type": "paragraph" }] },
                { "type": "tableCell", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "cell" }] }] }
            ]}]}
        ]));
        let result = sanitize_document(&input).unwrap();
        assert!(result.text.contains("Deploy") && result.text.contains("cargo run") && result.text.contains("cell"));
        assert!(result.text.contains("diagram"), "image alt text is searchable");
        // Sanitising is idempotent.
        assert_eq!(sanitize_document(&result.json).unwrap(), result);
    }

    #[test]
    fn rejects_unknown_nodes_and_marks() {
        let script = doc(json!([{ "type": "script", "content": [{ "type": "text", "text": "alert(1)" }] }]));
        assert!(sanitize_document(&script).is_err());

        let iframe =
            doc(json!([{ "type": "paragraph", "content": [{ "type": "iframe", "attrs": { "src": "https://x" } }] }]));
        assert!(sanitize_document(&iframe).is_err());

        let mark = doc(
            json!([{ "type": "paragraph", "content": [{ "type": "text", "text": "x", "marks": [{ "type": "textStyle", "attrs": { "style": "x" } }] }] }]),
        );
        assert!(sanitize_document(&mark).is_err());
    }

    #[test]
    fn rejects_malformed_documents() {
        assert!(sanitize_document(&json!("<p>html</p>")).is_err());
        assert!(sanitize_document(&json!({ "type": "paragraph" })).is_err());
        assert!(sanitize_document(&doc(json!("nope"))).is_err());
        assert!(sanitize_document(&doc(json!([{ "type": "doc" }]))).is_err(), "doc is only valid at the root");
        assert!(sanitize_document(&doc(json!([{ "type": "heading", "attrs": { "level": 9 } }]))).is_err());
    }

    #[test]
    fn unsafe_links_lose_the_mark_but_keep_the_text() {
        for href in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "data:text/html,<script>",
            "vbscript:x",
            "/relative",
            "java\nscript:alert(1)",
        ] {
            let input = doc(json!([{ "type": "paragraph", "content": [
                { "type": "text", "text": "click", "marks": [{ "type": "link", "attrs": { "href": href } }] }
            ]}]));
            let result = sanitize_document(&input).unwrap();
            assert_eq!(result.text, "click");
            assert!(!result.json.to_string().contains("link"), "{href} must not survive");
        }
    }

    #[test]
    fn link_attributes_are_normalised() {
        let input = doc(json!([{ "type": "paragraph", "content": [
            { "type": "text", "text": "x", "marks": [{ "type": "link", "attrs": {
                "href": "mailto:dev@example.com", "target": "_self", "rel": "opener", "onclick": "evil()", "class": "x"
            }}] }
        ]}]));
        let json = sanitize_document(&input).unwrap().json;
        let attrs = &json["content"][0]["content"][0]["marks"][0]["attrs"];
        assert_eq!(
            attrs,
            &json!({ "href": "mailto:dev@example.com", "target": "_blank", "rel": "noopener noreferrer nofollow" })
        );
    }

    #[test]
    fn unsafe_images_are_dropped_and_unknown_attrs_stripped() {
        let input = doc(json!([
            { "type": "image", "attrs": { "src": "data:image/svg+xml;base64,AAAA" } },
            { "type": "image", "attrs": { "src": "javascript:alert(1)" } },
            { "type": "image", "attrs": { "src": "https://example.com/ok.png", "onerror": "evil()", "style": "x" } },
            { "type": "paragraph", "attrs": { "style": "position:fixed", "class": "x" }, "content": [{ "type": "text", "text": "hi" }] }
        ]));
        let json = sanitize_document(&input).unwrap().json;
        let content = json["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["attrs"], json!({ "src": "https://example.com/ok.png" }));
        assert!(content[1].get("attrs").is_none());
    }

    #[test]
    fn list_numbering_and_cell_alignment_accept_only_known_values() {
        let input = doc(json!([
            { "type": "orderedList", "attrs": { "start": 2, "type": "a" }, "content": [{ "type": "listItem", "content": [{ "type": "paragraph" }] }] },
            { "type": "orderedList", "attrs": { "type": "expression(alert(1))" } },
            { "type": "table", "content": [{ "type": "tableRow", "content": [
                { "type": "tableCell", "attrs": { "align": "center" } },
                { "type": "tableCell", "attrs": { "align": "justify; position: fixed" } }
            ]}]}
        ]));
        let json = sanitize_document(&input).unwrap().json;
        assert_eq!(json["content"][0]["attrs"], json!({ "start": 2, "type": "a" }));
        assert_eq!(json["content"][1]["attrs"], json!({ "start": 1, "type": null }));
        let cells = &json["content"][2]["content"][0]["content"];
        assert_eq!(cells[0]["attrs"]["align"], "center");
        assert_eq!(cells[1]["attrs"]["align"], Value::Null);
    }

    #[test]
    fn code_block_language_is_restricted() {
        let input = doc(json!([{ "type": "codeBlock", "attrs": { "language": "rust\" onload=\"x" } }]));
        assert_eq!(sanitize_document(&input).unwrap().json["content"][0]["attrs"]["language"], Value::Null);
    }

    #[test]
    fn enforces_depth_and_size_limits() {
        let mut nested = json!({ "type": "paragraph" });
        for _ in 0..(MAX_DEPTH + 2) {
            nested = json!({ "type": "blockquote", "content": [nested] });
        }
        assert!(sanitize_document(&doc(json!([nested]))).is_err());

        let many: Vec<Value> = (0..=MAX_NODES).map(|_| json!({ "type": "horizontalRule" })).collect();
        assert!(sanitize_document(&doc(Value::Array(many))).is_err());
    }

    #[test]
    fn plain_text_projection_separates_blocks() {
        let input = doc(json!([
            { "type": "paragraph", "content": [{ "type": "text", "text": "first" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "" }, { "type": "text", "text": "second" }] }
        ]));
        assert_eq!(text_of(input), "first\nsecond");
        assert_eq!(text_of(empty_document()), "");
    }
}
