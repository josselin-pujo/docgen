//! Tiny typed wrapper over quick-xml's writer: element helpers with escaped
//! attributes and text, so emitters never hand-build strings.

use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Writer;

use crate::Error;

pub struct Xml {
    writer: Writer<Vec<u8>>,
}

impl Default for Xml {
    fn default() -> Self {
        Self::new()
    }
}

impl Xml {
    pub fn new() -> Self {
        Self {
            writer: Writer::new(Vec::new()),
        }
    }

    pub fn start(&mut self, name: &str, attrs: &[(&str, &str)]) -> &mut Self {
        let mut el = BytesStart::new(name);
        for (k, v) in attrs {
            el.push_attribute((*k, esc_attr(v).as_str()));
        }
        self.write(Event::Start(el));
        self
    }

    pub fn end(&mut self, name: &str) -> &mut Self {
        self.write(Event::End(BytesEnd::new(name)));
        self
    }

    /// Self-closing element.
    pub fn empty(&mut self, name: &str, attrs: &[(&str, &str)]) -> &mut Self {
        let mut el = BytesStart::new(name);
        for (k, v) in attrs {
            el.push_attribute((*k, esc_attr(v).as_str()));
        }
        self.write(Event::Empty(el));
        self
    }

    /// Escaped text content.
    pub fn text(&mut self, text: &str) -> &mut Self {
        self.write(Event::Text(BytesText::new(text)));
        self
    }

    fn write(&mut self, event: Event<'_>) {
        self.writer
            .write_event(event)
            .map_err(|e| Error::Xml(e.to_string()))
            .expect("in-memory XML writing cannot fail");
    }

    pub fn finish(self) -> Vec<u8> {
        self.writer.into_inner()
    }
}

/// Escapes an XML attribute value (`& < > "`).
pub fn esc_attr(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}
