//! A small, forgiving XML parser producing a lightweight DOM, plus escaping helpers for writers.
//!
//! Namespace prefixes are normalised: element and attribute names whose namespace URI is known
//! (OOXML, ODF, …) are rewritten to a canonical prefix (`w:`, `r:`, `text:` …) so readers can match on
//! fixed names regardless of the prefixes a producer chose.

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

impl Element {
    pub fn new(name: &str) -> Self {
        Element { name: name.to_string(), ..Default::default() }
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Local part of the name (after the prefix).
    pub fn local(&self) -> &str {
        self.name.rsplit(':').next().unwrap_or(&self.name)
    }

    pub fn is(&self, name: &str) -> bool {
        self.name == name
    }

    /// Child elements.
    pub fn elems(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elems().find(|e| e.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.elems().filter(move |e| e.name == name)
    }

    /// First descendant (depth-first, excluding self) with this name.
    pub fn find(&self, name: &str) -> Option<&Element> {
        for e in self.elems() {
            if e.name == name {
                return Some(e);
            }
            if let Some(f) = e.find(name) {
                return Some(f);
            }
        }
        None
    }

    /// All descendants with this name (depth-first order).
    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a Element>) {
        for e in self.elems() {
            if e.name == name {
                out.push(e);
            }
            e.find_all(name, out);
        }
    }

    /// Concatenated text of all descendant text nodes.
    pub fn text(&self) -> String {
        let mut s = String::new();
        self.collect_text(&mut s);
        s
    }

    fn collect_text(&self, s: &mut String) {
        for n in &self.children {
            match n {
                Node::Text(t) => s.push_str(t),
                Node::Element(e) => e.collect_text(s),
            }
        }
    }

    /// Value of `child/@attr`.
    pub fn child_attr(&self, child: &str, attr: &str) -> Option<&str> {
        self.child(child).and_then(|c| c.attr(attr))
    }
}

/// Known namespaces → canonical prefix.
const NAMESPACES: &[(&str, &str)] = &[
    ("http://schemas.openxmlformats.org/wordprocessingml/2006/main", "w"),
    ("http://purl.oclc.org/ooxml/wordprocessingml/main", "w"),
    ("http://schemas.openxmlformats.org/officeDocument/2006/relationships", "r"),
    ("http://purl.oclc.org/ooxml/officeDocument/relationships", "r"),
    ("http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing", "wp"),
    ("http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing", "wp"),
    ("http://schemas.openxmlformats.org/drawingml/2006/main", "a"),
    ("http://purl.oclc.org/ooxml/drawingml/main", "a"),
    ("http://schemas.openxmlformats.org/drawingml/2006/picture", "pic"),
    ("http://purl.oclc.org/ooxml/drawingml/picture", "pic"),
    ("http://schemas.openxmlformats.org/markup-compatibility/2006", "mc"),
    ("urn:schemas-microsoft-com:vml", "v"),
    ("urn:schemas-microsoft-com:office:office", "o"),
    ("urn:schemas-microsoft-com:office:word", "w10"),
    ("http://schemas.microsoft.com/office/word/2010/wordml", "w14"),
    ("http://schemas.microsoft.com/office/word/2012/wordml", "w15"),
    ("http://schemas.microsoft.com/office/word/2010/wordprocessingShape", "wps"),
    ("http://schemas.microsoft.com/office/word/2010/wordprocessingGroup", "wpg"),
    ("http://schemas.openxmlformats.org/officeDocument/2006/math", "m"),
    ("http://schemas.openxmlformats.org/package/2006/relationships", "pr"),
    ("http://schemas.openxmlformats.org/package/2006/content-types", "ct"),
    ("http://schemas.openxmlformats.org/package/2006/metadata/core-properties", "cp"),
    ("http://purl.org/dc/elements/1.1/", "dc"),
    ("http://purl.org/dc/terms/", "dcterms"),
    ("urn:oasis:names:tc:opendocument:xmlns:office:1.0", "office"),
    ("urn:oasis:names:tc:opendocument:xmlns:style:1.0", "style"),
    ("urn:oasis:names:tc:opendocument:xmlns:text:1.0", "text"),
    ("urn:oasis:names:tc:opendocument:xmlns:table:1.0", "table"),
    ("urn:oasis:names:tc:opendocument:xmlns:drawing:1.0", "draw"),
    ("urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0", "fo"),
    ("http://www.w3.org/1999/xlink", "xlink"),
    ("urn:oasis:names:tc:opendocument:xmlns:meta:1.0", "meta"),
    ("urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0", "svg"),
    ("urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0", "number"),
    ("urn:oasis:names:tc:opendocument:xmlns:manifest:1.0", "manifest"),
    ("urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0", "loext"),
    ("http://www.w3.org/XML/1998/namespace", "xml"),
];

fn canonical_prefix(uri: &str) -> Option<&'static str> {
    NAMESPACES.iter().find(|(u, _)| *u == uri).map(|(_, p)| *p)
}

struct Parser<'a> {
    s: &'a str,
    pos: usize,
}

/// Decode XML bytes (UTF-8 with optional BOM, or UTF-16 with BOM) to a string.
pub fn decode_bytes(b: &[u8]) -> String {
    if b.len() >= 2 && (b[0] == 0xFF && b[1] == 0xFE || b[0] == 0xFE && b[1] == 0xFF) {
        let le = b[0] == 0xFF;
        let units: Vec<u16> = b[2..]
            .chunks_exact(2)
            .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
            .collect();
        return String::from_utf16_lossy(&units);
    }
    let b = b.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(b);
    String::from_utf8_lossy(b).into_owned()
}

/// Parse an XML document from bytes.
pub fn parse_bytes(b: &[u8]) -> Result<Element, String> {
    parse(&decode_bytes(b))
}

/// Parse an XML document and return its root element.
pub fn parse(s: &str) -> Result<Element, String> {
    let mut p = Parser { s, pos: 0 };
    // Stack of (element, namespace scope pushed?)
    let mut stack: Vec<Element> = vec![Element::new("#document")];
    let mut scopes: Vec<Vec<(String, String)>> = vec![vec![]];
    let bytes = s.as_bytes();
    while p.pos < bytes.len() {
        if bytes[p.pos] == b'<' {
            let rest = &s[p.pos..];
            if rest.starts_with("<?") {
                p.skip_past("?>");
            } else if rest.starts_with("<!--") {
                p.skip_past("-->");
            } else if rest.starts_with("<![CDATA[") {
                let start = p.pos + 9;
                let end = s[start..].find("]]>").map(|i| start + i).unwrap_or(s.len());
                push_text(stack.last_mut().unwrap(), &s[start..end]);
                p.pos = (end + 3).min(s.len());
            } else if rest.starts_with("<!") {
                // DOCTYPE, possibly with an internal subset.
                let mut depth = 0i32;
                while p.pos < bytes.len() {
                    match bytes[p.pos] {
                        b'[' => depth += 1,
                        b']' => depth -= 1,
                        b'>' if depth <= 0 => {
                            p.pos += 1;
                            break;
                        }
                        _ => {}
                    }
                    p.pos += 1;
                }
            } else if rest.starts_with("</") {
                let end = s[p.pos..].find('>').map(|i| p.pos + i).unwrap_or(s.len());
                p.pos = (end + 1).min(s.len());
                if stack.len() > 1 {
                    let e = stack.pop().unwrap();
                    scopes.pop();
                    stack.last_mut().unwrap().children.push(Node::Element(e));
                }
            } else {
                // Start tag.
                p.pos += 1;
                let name = p.read_name();
                let mut raw_attrs: Vec<(String, String)> = Vec::new();
                let mut self_closing = false;
                loop {
                    p.skip_ws();
                    if p.pos >= bytes.len() {
                        break;
                    }
                    match bytes[p.pos] {
                        b'/' => {
                            self_closing = true;
                            p.pos += 1;
                        }
                        b'>' => {
                            p.pos += 1;
                            break;
                        }
                        _ => {
                            let an = p.read_name();
                            if an.is_empty() {
                                p.pos += 1;
                                continue;
                            }
                            p.skip_ws();
                            let mut val = String::new();
                            if p.pos < bytes.len() && bytes[p.pos] == b'=' {
                                p.pos += 1;
                                p.skip_ws();
                                if p.pos < bytes.len() && (bytes[p.pos] == b'"' || bytes[p.pos] == b'\'') {
                                    let q = bytes[p.pos] as char;
                                    let start = p.pos + 1;
                                    let end = s[start..].find(q).map(|i| start + i).unwrap_or(s.len());
                                    val = unescape(&s[start..end]);
                                    p.pos = (end + 1).min(s.len());
                                } else {
                                    let start = p.pos;
                                    while p.pos < bytes.len() && !bytes[p.pos].is_ascii_whitespace() && bytes[p.pos] != b'>' {
                                        p.pos += 1;
                                    }
                                    val = unescape(&s[start..p.pos]);
                                }
                            }
                            raw_attrs.push((an, val));
                        }
                    }
                }
                // Namespace scope.
                let mut scope = scopes.last().cloned().unwrap_or_default();
                for (k, v) in &raw_attrs {
                    if k == "xmlns" {
                        scope.retain(|(p, _)| !p.is_empty());
                        scope.push((String::new(), v.clone()));
                    } else if let Some(pfx) = k.strip_prefix("xmlns:") {
                        scope.retain(|(p, _)| p != pfx);
                        scope.push((pfx.to_string(), v.clone()));
                    }
                }
                let resolve = |n: &str, is_attr: bool| -> String {
                    let (pfx, local) = match n.split_once(':') {
                        Some((a, b)) => (a, b),
                        None => ("", n),
                    };
                    if pfx == "xml" {
                        return n.to_string();
                    }
                    if pfx.is_empty() && is_attr {
                        return n.to_string();
                    }
                    let uri = scope.iter().rev().find(|(p, _)| p == pfx).map(|(_, u)| u.as_str());
                    match uri.and_then(canonical_prefix) {
                        Some(c) => format!("{}:{}", c, local),
                        None => n.to_string(),
                    }
                };
                let el = Element {
                    name: resolve(&name, false),
                    attrs: raw_attrs
                        .iter()
                        .filter(|(k, _)| k != "xmlns" && !k.starts_with("xmlns:"))
                        .map(|(k, v)| (resolve(k, true), v.clone()))
                        .collect(),
                    children: Vec::new(),
                };
                if self_closing {
                    stack.last_mut().unwrap().children.push(Node::Element(el));
                } else {
                    stack.push(el);
                    scopes.push(scope);
                }
            }
        } else {
            let end = s[p.pos..].find('<').map(|i| p.pos + i).unwrap_or(s.len());
            let raw = &s[p.pos..end];
            if stack.len() > 1 {
                push_text(stack.last_mut().unwrap(), &unescape(raw));
            }
            p.pos = end;
        }
    }
    // Close unclosed elements.
    while stack.len() > 1 {
        let e = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(Node::Element(e));
    }
    let doc = stack.pop().unwrap();
    doc.children
        .into_iter()
        .find_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
        .ok_or_else(|| "XML document has no root element".to_string())
}

fn push_text(e: &mut Element, t: &str) {
    if t.is_empty() {
        return;
    }
    if let Some(Node::Text(prev)) = e.children.last_mut() {
        prev.push_str(t);
    } else {
        e.children.push(Node::Text(t.to_string()));
    }
}

impl Parser<'_> {
    fn skip_past(&mut self, pat: &str) {
        match self.s[self.pos..].find(pat) {
            Some(i) => self.pos += i + pat.len(),
            None => self.pos = self.s.len(),
        }
    }
    fn skip_ws(&mut self) {
        let b = self.s.as_bytes();
        while self.pos < b.len() && b[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }
    fn read_name(&mut self) -> String {
        let b = self.s.as_bytes();
        let start = self.pos;
        while self.pos < b.len() {
            let c = b[self.pos];
            if c.is_ascii_whitespace() || c == b'>' || c == b'/' || c == b'=' {
                break;
            }
            self.pos += 1;
        }
        self.s[start..self.pos].to_string()
    }
}

/// Decode XML entity and character references.
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.bytes().take(12).position(|b| b == b';');
        match end {
            Some(e) => {
                let ent = &rest[1..e];
                let ch = match ent {
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "amp" => Some('&'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                        u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32)
                    }
                    _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
                    _ => None,
                };
                match ch {
                    Some(c) => {
                        out.push(c);
                        rest = &rest[e + 1..];
                    }
                    None => {
                        out.push('&');
                        rest = &rest[1..];
                    }
                }
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn valid_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r') || (c >= ' ' && c != '\u{FFFE}' && c != '\u{FFFF}')
}

/// Escape text content (also drops characters not allowed in XML 1.0).
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            c if valid_xml_char(c) => out.push(c),
            _ => {}
        }
    }
    out
}

/// Escape an attribute value (double-quoted).
pub fn esc_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            c if valid_xml_char(c) => out.push(c),
            _ => {}
        }
    }
    out
}

/// Simple attribute map lookup helper used by writers' tests.
pub fn attr_map(e: &Element) -> HashMap<&str, &str> {
    e.attrs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_namespaces_and_text() {
        let x = r#"<?xml version="1.0"?><x:document xmlns:x="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><x:body><x:t xml:space="preserve"> a &amp; b &#x41;</x:t><![CDATA[<c>]]></x:body></x:document>"#;
        let d = parse(x).unwrap();
        assert_eq!(d.name, "w:document");
        let b = d.child("w:body").unwrap();
        assert_eq!(b.child("w:t").unwrap().text(), " a & b A");
        assert_eq!(b.child("w:t").unwrap().attr("xml:space"), Some("preserve"));
        assert_eq!(b.text(), " a & b A<c>");
    }
}
