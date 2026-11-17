//! A scanner for Resolve's project XML that records where each element sits in the text. Edits are
//! splices into the original text, so everything an edit doesn't touch stays exactly as Resolve wrote it.
//! Offsets are byte offsets; every one sits at an ASCII `<` or `>`, so slicing there is safe.

use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::{Error, Result, fail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlElement {
    pub name: String,
    /// DbId attribute, which every Resolve object has.
    pub id: Option<String>,
    /// Offsets into the text: the whole element, and what's between its tags.
    pub start: usize,
    pub end: usize,
    pub inner_start: usize,
    pub inner_end: usize,
    pub children: Vec<XmlElement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Splice {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

pub fn parse_xml(text: &str) -> Result<XmlElement> {
    let bytes = text.as_bytes();
    let document = XmlElement {
        name: String::new(),
        id: None,
        start: 0,
        end: text.len(),
        inner_start: 0,
        inner_end: text.len(),
        children: Vec::new(),
    };
    let mut open = vec![document];
    let mut next = text.find('<');
    while let Some(at) = next {
        let rest = &text[at..];
        let after = if rest.starts_with("<?") {
            skip(text, at, "?>")?
        } else if rest.starts_with("<!--") {
            skip(text, at, "-->")?
        } else if rest.starts_with("<![CDATA[") {
            skip(text, at, "]]>")?
        } else if rest.starts_with("<!") {
            skip(text, at, ">")?
        } else if bytes.get(at + 1) == Some(&b'/') {
            let close = skip(text, at, ">")?;
            let name = text[at + 2..close - 1].trim();
            if open.len() < 2 || open.last().is_some_and(|element| element.name != name) {
                return fail(format!("Unexpected </{name}> at {at}."));
            }
            let mut element = open.pop().expect("checked above");
            element.inner_end = at;
            element.end = close;
            open.last_mut()
                .expect("checked above")
                .children
                .push(element);
            close
        } else {
            let close = tag_end(text, at)?;
            let self_closing = bytes[close - 2] == b'/';
            let tag = &text[at + 1..if self_closing { close - 2 } else { close - 1 }];
            let name_end = tag
                .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
                .unwrap_or(tag.len());
            let element = XmlElement {
                name: tag[..name_end].to_string(),
                id: db_id(tag),
                start: at,
                end: close,
                inner_start: close,
                inner_end: close,
                children: Vec::new(),
            };
            if self_closing {
                open.last_mut()
                    .expect("the document is always open")
                    .children
                    .push(element);
            } else {
                open.push(element);
            }
            close
        };
        next = text[after..].find('<').map(|found| after + found);
    }
    if open.len() != 1 {
        return fail(format!(
            "<{}> is never closed.",
            open.last().expect("not empty").name
        ));
    }
    let document = open.pop().expect("not empty");
    document
        .children
        .into_iter()
        .next()
        .ok_or_else(|| Error("The XML has no root element.".into()))
}

/// The tag's `DbId="…"` value, when the attribute follows whitespace.
fn db_id(tag: &str) -> Option<String> {
    let mut from = 0;
    while let Some(found) = tag[from..].find("DbId=\"") {
        let at = from + found;
        let value = at + "DbId=\"".len();
        if tag[..at]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
            && let Some(length) = tag[value..].find('"')
        {
            return Some(tag[value..value + length].to_string());
        }
        from = value;
    }
    None
}

fn skip(text: &str, at: usize, terminator: &str) -> Result<usize> {
    match text[at..].find(terminator) {
        Some(found) => Ok(at + found + terminator.len()),
        None => fail(format!("Unterminated markup at {at}.")),
    }
}

/// The index after a start tag's `>`, stepping over quoted attribute values.
fn tag_end(text: &str, at: usize) -> Result<usize> {
    let mut quote = 0u8;
    for (i, &char) in text.as_bytes().iter().enumerate().skip(at + 1) {
        if quote != 0 {
            if char == quote {
                quote = 0;
            }
        } else if char == b'"' || char == b'\'' {
            quote = char;
        } else if char == b'>' {
            return Ok(i + 1);
        }
    }
    fail(format!("Unterminated tag at {at}."))
}

pub fn child<'a>(element: &'a XmlElement, name: &str) -> Option<&'a XmlElement> {
    element
        .children
        .iter()
        .find(|candidate| candidate.name == name)
}

/// Every element below `element`, depth first.
pub fn descendants(element: &XmlElement) -> Vec<&XmlElement> {
    let mut all = Vec::new();
    fn walk<'a>(element: &'a XmlElement, all: &mut Vec<&'a XmlElement>) {
        for next in &element.children {
            all.push(next);
            walk(next, all);
        }
    }
    walk(element, &mut all);
    all
}

/// The element's text with entities decoded, or "" when it has children or is empty.
pub fn text_of(text: &str, element: Option<&XmlElement>) -> String {
    match element {
        Some(element) if element.children.is_empty() => {
            unescape(&text[element.inner_start..element.inner_end])
        }
        _ => String::new(),
    }
}

/// The text of `element`'s child called `name`.
pub fn child_text(text: &str, element: &XmlElement, name: &str) -> String {
    text_of(text, child(element, name))
}

/// A splice that sets a leaf element's text, keeping the tag. An empty value writes `<Name/>`, like Resolve.
pub fn set_text(text: &str, element: &XmlElement, value: &str) -> Result<Splice> {
    if !element.children.is_empty() {
        return fail(format!("<{}> isn't a text element.", element.name));
    }
    let self_closing = element.inner_start == element.end;
    let tag = if self_closing {
        text[element.start + 1..element.end - 2].trim_end()
    } else {
        &text[element.start + 1..element.inner_start - 1]
    };
    Ok(Splice {
        start: element.start,
        end: element.end,
        text: if value.is_empty() {
            format!("<{tag}/>")
        } else {
            format!("<{tag}>{}</{}>", escape(value), element.name)
        },
    })
}

/// `text` with the splices applied. They mustn't overlap.
pub fn apply_splices(text: &str, splices: &[Splice]) -> Result<String> {
    let mut ordered: Vec<&Splice> = splices.iter().collect();
    ordered.sort_by_key(|splice| std::cmp::Reverse(splice.start));
    let mut result = text.to_string();
    let mut limit = usize::MAX;
    for splice in ordered {
        if splice.end > limit {
            return fail("Two edits touch the same part of the XML.");
        }
        result.replace_range(splice.start..splice.end, &splice.text);
        limit = splice.start;
    }
    Ok(result)
}

static ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)&(#x[0-9a-f]+|#[0-9]+|[0-9a-z_]+);").unwrap());

fn unescape(value: &str) -> String {
    ENTITY
        .replace_all(value, |captures: &Captures| {
            let whole = &captures[0];
            let entity = &captures[1];
            let decoded = if let Some(number) = entity.strip_prefix('#') {
                let code = match number.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => number.parse().ok(),
                };
                code.and_then(char::from_u32).map(String::from)
            } else {
                match entity {
                    "amp" => Some("&".into()),
                    "lt" => Some("<".into()),
                    "gt" => Some(">".into()),
                    "quot" => Some("\"".into()),
                    "apos" => Some("'".into()),
                    _ => None,
                }
            };
            decoded.unwrap_or_else(|| whole.to_string())
        })
        .into_owned()
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
