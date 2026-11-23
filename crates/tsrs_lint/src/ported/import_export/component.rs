//! The `<script>` blocks of a Vue component, as `@vue/compiler-sfc` finds them: `js/import-export` reads the
//! exports of a `.vue` file that an `export * from` names from the text of these blocks.
//!
//! Only what decides where a root `<script>` starts and ends is read: the root tags and their attributes, comments,
//! and the content of the root `<template>`, in which a `<script>` is an element of the template.

/// A root `<script>` block.
pub(super) struct ScriptBlock<'a> {
    pub(super) content: &'a str,
    /// The value of `lang`, when the attribute has one.
    pub(super) lang: Option<&'a str>,
    /// True for a `src` with a value that is not empty.
    src: bool,
}

/// The attributes of a tag that the blocks depend on, and where the tag ends.
struct Tag<'a> {
    name: &'a str,
    lang: Option<&'a str>,
    has_src: bool,
    src: bool,
    setup: bool,
    self_closing: bool,
    /// The offset after `>`.
    end: usize,
}

fn is_tag_end(byte: u8) -> bool {
    byte.is_ascii_whitespace() || byte == b'/' || byte == b'>'
}

/// The tag that starts at `start` (a `<` followed by a letter).
fn open_tag(source: &str, start: usize) -> Tag<'_> {
    let bytes = source.as_bytes();
    let mut at = start + 1;
    while at < bytes.len() && !is_tag_end(bytes[at]) {
        at += 1;
    }
    let mut tag = Tag {
        name: &source[start + 1..at],
        lang: None,
        has_src: false,
        src: false,
        setup: false,
        self_closing: false,
        end: bytes.len(),
    };
    loop {
        while at < bytes.len() && (bytes[at].is_ascii_whitespace() || bytes[at] == b'/') {
            tag.self_closing = bytes[at] == b'/';
            at += 1;
        }
        if at >= bytes.len() {
            return tag;
        }
        if bytes[at] == b'>' {
            tag.end = at + 1;
            return tag;
        }
        tag.self_closing = false;
        let name_start = at;
        while at < bytes.len() && !is_tag_end(bytes[at]) && bytes[at] != b'=' {
            at += 1;
        }
        let name = &source[name_start..at];
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let mut value = None;
        if at < bytes.len() && bytes[at] == b'=' {
            at += 1;
            while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                at += 1;
            }
            let value_start = at;
            if at < bytes.len() && (bytes[at] == b'"' || bytes[at] == b'\'') {
                let quote = bytes[at];
                at += 1;
                while at < bytes.len() && bytes[at] != quote {
                    at += 1;
                }
                value = Some(&source[value_start + 1..at]);
                at = (at + 1).min(bytes.len());
            } else {
                while at < bytes.len() && !bytes[at].is_ascii_whitespace() && bytes[at] != b'>' {
                    at += 1;
                }
                value = Some(&source[value_start..at]);
            }
        }
        match name {
            "lang" => tag.lang = value,
            "src" => {
                tag.has_src = true;
                tag.src = value.is_some_and(|value| !value.is_empty());
            }
            "setup" => tag.setup = true,
            _ => {}
        }
    }
}

/// Where the text that follows an open tag `name` ends, and the offset after its closing tag: the closing tag is
/// the first `</name` before a `>` or a space, in any case.
fn raw_text_end(source: &str, from: usize, name: &str) -> (usize, usize) {
    let bytes = source.as_bytes();
    let mut at = from;
    while let Some(found) = source.get(at..).and_then(|rest| rest.find("</")) {
        let close = at + found;
        let after = close + 2 + name.len();
        if source.get(close + 2..after).is_some_and(|text| text.eq_ignore_ascii_case(name))
            && bytes.get(after).is_none_or(|&byte| byte == b'>' || byte.is_ascii_whitespace())
        {
            let end = source[after..].find('>').map_or(bytes.len(), |index| after + index + 1);
            return (close, end);
        }
        at = close + 2;
    }
    (bytes.len(), bytes.len())
}

/// The offset of the first `wanted` at or after `from`.
fn find(bytes: &[u8], from: usize, wanted: &[u8]) -> Option<usize> {
    let position = bytes.get(from..)?.windows(wanted.len()).position(|window| window == wanted);
    position.map(|index| from + index)
}

/// The offset after the `</template>` that closes a root `<template>` whose content starts at `from`.
fn template_end(source: &str, from: usize) -> usize {
    let bytes = source.as_bytes();
    let after = |at, wanted: &[u8]| {
        find(bytes, at, wanted).map_or(bytes.len(), |index| index + wanted.len())
    };
    let mut depth = 1_usize;
    let mut at = from;
    while at < bytes.len() {
        let rest = &bytes[at..];
        if rest.starts_with(b"{{") {
            at = after(at, b"}}");
        } else if rest.starts_with(b"<!--") {
            at = after(at, b"-->");
        } else if rest.starts_with(b"</") {
            let name_end = rest.iter().position(|&byte| byte == b'>' || byte.is_ascii_whitespace());
            let closes_template = rest.get(2..name_end.unwrap_or(rest.len())) == Some(b"template");
            at = after(at, b">");
            if closes_template {
                depth -= 1;
                if depth == 0 {
                    return at;
                }
            }
        } else if rest[0] == b'<' && rest.get(1).is_some_and(u8::is_ascii_alphabetic) {
            let tag = open_tag(source, at);
            at = tag.end;
            if tag.self_closing {
                continue;
            }
            if tag.name == "template" {
                depth += 1;
            } else if ["script", "style", "textarea", "title"]
                .iter()
                .any(|raw| tag.name.eq_ignore_ascii_case(raw))
            {
                at = raw_text_end(source, at, tag.name).1;
            }
        } else {
            at += 1;
        }
    }
    bytes.len()
}

/// The `<script>` and the `<script setup>` of a component, in that order: the first block of each kind, without
/// the empty ones.
pub(super) fn script_blocks(source: &str) -> [Option<ScriptBlock<'_>>; 2] {
    let bytes = source.as_bytes();
    let mut script = None;
    let mut script_setup = None;
    let mut at = 0;
    while let Some(found) = source.get(at..).and_then(|rest| rest.find('<')) {
        at += found;
        let rest = &source[at..];
        if rest.starts_with("<!--") {
            at = rest.find("-->").map_or(bytes.len(), |index| at + index + 3);
        } else if bytes.get(at + 1).is_some_and(|&byte| matches!(byte, b'/' | b'!' | b'?')) {
            at = rest.find('>').map_or(bytes.len(), |index| at + index + 1);
        } else if bytes.get(at + 1).is_some_and(u8::is_ascii_alphabetic) {
            let tag = open_tag(source, at);
            let (content_end, end) = if tag.self_closing {
                (tag.end, tag.end)
            } else if tag.name == "template" && tag.lang.is_none_or(|lang| lang == "html") {
                let end = template_end(source, tag.end);
                (end, end)
            } else {
                raw_text_end(source, tag.end, tag.name)
            };
            at = end;
            let content = &source[tag.end..content_end];
            if tag.name == "script" && (tag.has_src || !content.trim().is_empty()) {
                let slot = if tag.setup { &mut script_setup } else { &mut script };
                if slot.is_none() {
                    *slot = Some(ScriptBlock { content, lang: tag.lang, src: tag.src });
                }
            }
        } else {
            at += 1;
        }
    }
    // A `<script setup>` takes no `src`, and neither does the `<script>` next to one.
    if let Some(setup) = &script_setup {
        let drop_script = script.as_ref().is_some_and(|script| script.src);
        if setup.src {
            script_setup = None;
        }
        if drop_script {
            script = None;
        }
    }
    [script, script_setup]
}
