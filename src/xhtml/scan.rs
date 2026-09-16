//! Low-level, syntax-only XHTML/HTML scanning primitives.
//!
//! The scanner handles tag boundaries, names, attributes, and raw-text
//! elements. Semantic interpretation such as cover, stylesheet, layout, or
//! position-bearing status belongs to callers.

pub(crate) fn advance_char(source: &str, cursor: usize) -> usize {
    source
        .get(cursor..)
        .and_then(|remaining| remaining.chars().next())
        .map_or(source.len(), |character| cursor + character.len_utf8())
}

/// Find an ASCII needle without allocating a lowercased copy of `source`.
///
/// This has the same case-folding scope as `str::to_ascii_lowercase`: bytes
/// outside ASCII are compared unchanged and ASCII letters are compared
/// case-insensitively.
pub(crate) fn find_ascii_case_insensitive(
    source: &str,
    needle: &str,
    start: usize,
) -> Option<usize> {
    let needle = needle.as_bytes();
    if needle.is_empty() {
        return Some(start.min(source.len()));
    }
    source
        .as_bytes()
        .get(start..)?
        .windows(needle.len())
        .position(|window| ascii_bytes_eq_ignore_case(window, needle))
        .map(|relative| start + relative)
}

fn ascii_bytes_eq_ignore_case(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(&left, &right)| left.eq_ignore_ascii_case(&right))
}

pub(crate) fn html_tag_end(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut cursor = start + 1;
    let mut quote = None;
    while cursor < bytes.len() {
        match (quote, bytes[cursor]) {
            (Some(expected), byte) if byte == expected => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(bytes[cursor]),
            (None, b'>') => return Some(cursor),
            (None, _) => {}
        }
        cursor = advance_char(source, cursor);
    }
    None
}

pub(crate) fn html_tag_name_range(
    source: &str,
    start: usize,
    tag_end: usize,
) -> Option<(usize, usize, bool)> {
    let bytes = source.as_bytes();
    let mut cursor = start + 1;
    let closing = bytes.get(cursor) == Some(&b'/');
    if closing {
        cursor += 1;
    }
    if cursor >= tag_end || matches!(bytes[cursor], b'!' | b'?') {
        return None;
    }
    let name_start = cursor;
    while cursor < tag_end
        && !bytes[cursor].is_ascii_whitespace()
        && !matches!(bytes[cursor], b'/' | b'>')
    {
        cursor = advance_char(source, cursor);
    }
    (name_start < cursor).then_some((name_start, cursor, closing))
}

pub(crate) fn html_tag_name_range_with_leading_space(
    source: &str,
    start: usize,
    tag_end: usize,
) -> Option<(usize, usize, bool)> {
    let bytes = source.as_bytes();
    let mut cursor = start + 1;
    let closing = bytes.get(cursor) == Some(&b'/');
    if closing {
        cursor += 1;
    }
    while cursor < tag_end && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    if cursor >= tag_end || matches!(bytes[cursor], b'!' | b'?') {
        return None;
    }
    let name_start = cursor;
    while cursor < tag_end
        && !bytes[cursor].is_ascii_whitespace()
        && !matches!(bytes[cursor], b'/' | b'>')
    {
        cursor = advance_char(source, cursor);
    }
    (name_start < cursor).then_some((name_start, cursor, closing))
}

pub(crate) fn html_local_name_is(source: &str, start: usize, end: usize, wanted: &str) -> bool {
    source[start..end]
        .rsplit(':')
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
}

pub(crate) fn html_local_name_is_text(name: &str, wanted: &str) -> bool {
    name.rsplit(':')
        .next()
        .is_some_and(|local| local.eq_ignore_ascii_case(wanted))
}

pub(crate) fn html_raw_text_end(source: &str, start: usize, tag_end: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let (name_start, name_end, closing) = html_tag_name_range(source, start, tag_end)?;
    if closing {
        return None;
    }
    let mut content_end = tag_end;
    while content_end > start && bytes[content_end - 1].is_ascii_whitespace() {
        content_end -= 1;
    }
    if bytes.get(content_end - 1) == Some(&b'/') {
        return None;
    }
    let raw_name = if html_local_name_is(source, name_start, name_end, "script") {
        "script"
    } else if html_local_name_is(source, name_start, name_end, "style") {
        "style"
    } else {
        return None;
    };
    let mut cursor = tag_end + 1;
    let mut quote = None;
    let mut block_comment = false;
    let mut line_comment = false;
    let mut html_comment = false;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if html_comment {
            if byte == b'-'
                && bytes.get(cursor + 1) == Some(&b'-')
                && bytes.get(cursor + 2) == Some(&b'>')
            {
                html_comment = false;
                cursor += 3;
            } else {
                cursor = advance_char(source, cursor);
            }
            continue;
        }
        if block_comment {
            if byte == b'*' && bytes.get(cursor + 1) == Some(&b'/') {
                block_comment = false;
                cursor += 2;
            } else {
                cursor = advance_char(source, cursor);
            }
            continue;
        }
        if line_comment {
            if byte == b'\r' || byte == b'\n' {
                line_comment = false;
            }
            cursor = advance_char(source, cursor);
            continue;
        }
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                cursor = advance_char(source, cursor);
                if cursor < bytes.len() {
                    cursor = advance_char(source, cursor);
                }
            } else {
                if byte == delimiter {
                    quote = None;
                }
                cursor = advance_char(source, cursor);
            }
            continue;
        }
        if byte == b'<'
            && bytes.get(cursor + 1) == Some(&b'!')
            && bytes.get(cursor + 2) == Some(&b'-')
            && bytes.get(cursor + 3) == Some(&b'-')
        {
            html_comment = true;
            cursor += 4;
            continue;
        }
        if byte == b'/' && bytes.get(cursor + 1) == Some(&b'*') {
            block_comment = true;
            cursor += 2;
            continue;
        }
        if raw_name == "script" && byte == b'/' && bytes.get(cursor + 1) == Some(&b'/') {
            line_comment = true;
            cursor += 2;
            continue;
        }
        if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
            cursor = advance_char(source, cursor);
            continue;
        }
        if byte == b'<' && bytes.get(cursor + 1) == Some(&b'/') {
            let Some(candidate_end) = html_tag_end(source, cursor) else {
                return Some(source.len());
            };
            if let Some((candidate_start, candidate_name_end, candidate_closing)) =
                html_tag_name_range(source, cursor, candidate_end)
            {
                if candidate_closing
                    && html_local_name_is(source, candidate_start, candidate_name_end, raw_name)
                {
                    return Some(candidate_end + 1);
                }
            }
            cursor = candidate_end + 1;
            continue;
        }
        cursor = advance_char(source, cursor);
    }
    Some(source.len())
}

/// A syntax-only XHTML tag view with source-relative byte offsets.
///
/// The iterator deliberately keeps the historical KF8 tag-walk behavior:
/// non-element markup is skipped, quoted tag boundaries are respected, and
/// script/style payloads are consumed as raw text through their matching end
/// tag. Callers own semantic interpretation and may borrow the tag source
/// without materializing the complete document's tag list.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tag<'a> {
    pub(crate) source: &'a str,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) name_start: usize,
    pub(crate) name_end: usize,
}

impl Tag<'_> {
    pub(crate) fn name(&self) -> &str {
        &self.source[self.name_start..self.name_end]
    }

    pub(crate) fn attribute(&self, wanted: &str) -> Option<&str> {
        let bytes = self.source.as_bytes();
        let mut cursor = self.name_end;
        while cursor < self.end {
            while cursor < self.end && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if cursor >= self.end || bytes[cursor] == b'>' || bytes[cursor] == b'/' {
                break;
            }
            let name_start = cursor;
            while cursor < self.end
                && !bytes[cursor].is_ascii_whitespace()
                && !matches!(bytes[cursor], b'=' | b'>')
            {
                cursor += 1;
            }
            let name = &self.source[name_start..cursor];
            while cursor < self.end && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if cursor >= self.end || bytes[cursor] != b'=' {
                while cursor < self.end && bytes[cursor] != b'>' {
                    cursor += 1;
                }
                continue;
            }
            cursor += 1;
            while cursor < self.end && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            let quote = bytes.get(cursor).copied();
            let (value_start, value_end) = if matches!(quote, Some(b'"') | Some(b'\'')) {
                cursor += 1;
                let value_start = cursor;
                while cursor < self.end && bytes[cursor] != quote.unwrap() {
                    cursor += 1;
                }
                (value_start, cursor)
            } else {
                let value_start = cursor;
                while cursor < self.end
                    && !bytes[cursor].is_ascii_whitespace()
                    && bytes[cursor] != b'>'
                {
                    cursor += 1;
                }
                (value_start, cursor)
            };
            if name.eq_ignore_ascii_case(wanted) {
                return Some(&self.source[value_start..value_end]);
            }
            if cursor < self.end && quote.is_some() {
                cursor += 1;
            }
        }
        None
    }
}

pub(crate) struct Tags<'a> {
    source: &'a str,
    cursor: usize,
}

pub(crate) fn tags(source: &str) -> Tags<'_> {
    Tags { source, cursor: 0 }
}

impl<'a> Iterator for Tags<'a> {
    type Item = Tag<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let bytes = self.source.as_bytes();
        while let Some(relative) = bytes[self.cursor..].iter().position(|byte| *byte == b'<') {
            let start = self.cursor + relative;
            if let Some(next) = scanner_non_element_markup_end(bytes, start) {
                self.cursor = next;
                continue;
            }
            let end = html_tag_end(self.source, start)?;
            let mut name_start = start + 1;
            while name_start < end && matches!(bytes[name_start], b'/' | b'!' | b'?') {
                name_start += 1;
            }
            let mut name_end = name_start;
            while name_end < end
                && !bytes[name_end].is_ascii_whitespace()
                && !matches!(bytes[name_end], b'/' | b'>')
            {
                name_end += 1;
            }
            self.cursor = end + 1;
            if name_end <= name_start {
                continue;
            }
            let tag = Tag {
                source: self.source,
                start,
                end: end + 1,
                name_start,
                name_end,
            };
            let opening_tag = bytes.get(start + 1) != Some(&b'/');
            let raw_text = opening_tag
                && !bytes[start..=end].ends_with(b"/>")
                && (tag.name().eq_ignore_ascii_case("script")
                    || tag.name().eq_ignore_ascii_case("style"));
            if raw_text {
                self.cursor = scanner_raw_text_element_end(
                    bytes,
                    end + 1,
                    &self.source[name_start..name_end],
                )
                .map_or(bytes.len(), |(_, close_end)| close_end);
            }
            return Some(tag);
        }
        None
    }
}

fn scanner_non_element_markup_end(source: &[u8], start: usize) -> Option<usize> {
    let remainder = source.get(start..)?;
    if remainder.starts_with(b"<!--") {
        return find_bytes(remainder, 4, b"-->").map(|offset| start + offset + 3);
    }
    if remainder.starts_with(b"<![CDATA[") {
        return find_bytes(remainder, 9, b"]]>").map(|offset| start + offset + 3);
    }
    if remainder
        .get(1)
        .is_some_and(|byte| matches!(byte, b'!' | b'?'))
    {
        return html_tag_end(std::str::from_utf8(source).ok()?, start).map(|end| end + 1);
    }
    None
}

fn scanner_raw_text_element_end(source: &[u8], start: usize, name: &str) -> Option<(usize, usize)> {
    let name = name.as_bytes();
    let mut cursor = start;
    while cursor + 2 + name.len() <= source.len() {
        if source[cursor] == b'<'
            && source.get(cursor + 1) == Some(&b'/')
            && source
                .get(cursor + 2..cursor + 2 + name.len())
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
        {
            let boundary = source.get(cursor + 2 + name.len()).copied();
            if boundary.is_some_and(|byte| byte.is_ascii_whitespace() || byte == b'>') {
                let source = std::str::from_utf8(source).ok()?;
                let end = html_tag_end(source, cursor)?;
                return Some((cursor, end + 1));
            }
        }
        cursor += 1;
    }
    None
}

fn find_bytes(source: &[u8], start: usize, needle: &[u8]) -> Option<usize> {
    source
        .get(start..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| start + offset)
}
