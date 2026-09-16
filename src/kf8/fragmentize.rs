//! Fragment XHTML structurally into a reconstructable SKEL/FRAG representation.
//!
//! This module owns element context, fragment payload boundaries, and skeleton
//! insertion geometry. It does not define Kindle position coordinates; that
//! mapping is the responsibility of `position`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FragmentContext {
    pub selector: String,
    pub source_start: usize,
    pub source_end: usize,
    /// Offset in the mutated skeleton before this fragment is inserted.
    pub skeleton_offset: usize,
    pub starts_tags: Vec<String>,
    pub ends_tags: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct FragmentizedBody {
    pub skeleton: Vec<u8>,
    pub fragments: Vec<RawFragment>,
    pub contexts: Vec<FragmentContext>,
}

#[derive(Debug, Clone)]
pub(crate) struct RawFragment {
    pub raw: Vec<u8>,
    selector: String,
    source_start: usize,
    source_end: usize,
    starts_tags: Vec<String>,
    ends_tags: Vec<String>,
}

pub(crate) fn fragmentize_body(
    body: &[u8],
    body_aid: Option<&str>,
    threshold: usize,
) -> FragmentizedBody {
    if body.is_empty() {
        let aid = body_aid.unwrap_or("").to_owned();
        return FragmentizedBody {
            skeleton: Vec::new(),
            fragments: vec![RawFragment {
                raw: Vec::new(),
                selector: selector("P", body_aid.unwrap_or("")),
                source_start: 0,
                source_end: 0,
                starts_tags: vec![aid.clone()],
                ends_tags: vec![aid.clone()],
            }],
            contexts: vec![FragmentContext {
                selector: selector("P", &aid),
                source_start: 0,
                source_end: 0,
                skeleton_offset: 0,
                starts_tags: vec![aid.clone()],
                ends_tags: vec![aid],
            }],
        };
    }

    let nodes = parse_elements(body);
    let mut roots = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.parent.is_none())
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    roots.sort_unstable_by_key(|index| nodes[*index].start);

    let mut chunks = Vec::new();
    let mut removals = Vec::new();
    let root_aid = body_aid.unwrap_or("").to_owned();
    let pseudo_root = Element {
        start: 0,
        open_end: 0,
        close_start: body.len(),
        end: body.len(),
        name: "body".to_owned(),
        aid: Some(root_aid),
        parent: None,
        children: roots,
    };
    step_into_element(
        &pseudo_root,
        &nodes,
        body,
        threshold.max(1),
        &mut chunks,
        &mut removals,
    );

    if chunks.is_empty() {
        chunks.push(RawFragment {
            raw: Vec::new(),
            selector: selector("P", body_aid.unwrap_or("")),
            source_start: 0,
            source_end: 0,
            starts_tags: vec![body_aid.unwrap_or("").to_owned()],
            ends_tags: vec![body_aid.unwrap_or("").to_owned()],
        });
    }
    merge_fragment_chunks(&mut chunks, threshold.max(1));
    removals.sort_unstable_by_key(|range| (range.0, range.1));
    let mut skeleton = Vec::with_capacity(body.len());
    let mut cursor = 0;
    for (start, end) in &removals {
        if *start < cursor || *end > body.len() || start >= end {
            continue;
        }
        skeleton.extend_from_slice(&body[cursor..*start]);
        cursor = *end;
    }
    skeleton.extend_from_slice(&body[cursor..]);

    let mut removal_index = 0usize;
    let mut removed_before = 0usize;
    let contexts = chunks
        .iter()
        .map(|chunk| {
            // Several UTF-8 chunks can originate inside one removed text
            // range. They all insert at that range's single skeleton offset;
            // using each chunk's source offset would count the removed text
            // repeatedly and make the second insertion run past the skeleton.
            while let Some((start, end)) = removals.get(removal_index) {
                if *end > chunk.source_start {
                    break;
                }
                removed_before += end - start;
                removal_index += 1;
            }
            let anchor = removals
                .get(removal_index)
                .filter(|(start, end)| *start <= chunk.source_start && chunk.source_start < *end)
                .map(|(start, _)| *start)
                .unwrap_or(chunk.source_start);
            FragmentContext {
                selector: chunk.selector.clone(),
                source_start: chunk.source_start,
                source_end: chunk.source_end,
                skeleton_offset: anchor - removed_before,
                starts_tags: chunk.starts_tags.clone(),
                ends_tags: chunk.ends_tags.clone(),
            }
        })
        .collect();
    FragmentizedBody {
        skeleton,
        fragments: chunks,
        contexts,
    }
}

fn selector(kind: &str, aid: &str) -> String {
    format!("{kind}-//*[@aid='{aid}']")
}

#[derive(Debug, Clone)]
pub(super) struct Element {
    pub(super) start: usize,
    pub(super) open_end: usize,
    pub(super) close_start: usize,
    pub(super) end: usize,
    pub(super) name: String,
    pub(super) aid: Option<String>,
    pub(super) parent: Option<usize>,
    pub(super) children: Vec<usize>,
}

fn step_into_element(
    element: &Element,
    nodes: &[Element],
    source: &[u8],
    threshold: usize,
    chunks: &mut Vec<RawFragment>,
    removals: &mut Vec<(usize, usize)>,
) {
    let Some(aid) = element.aid.as_deref() else {
        return;
    };
    let first_chunk = chunks.len();
    let mut current_selector = selector("P", aid);
    let first_child = element
        .children
        .first()
        .map(|index| nodes[*index].start)
        .unwrap_or(element.close_start);
    emit_text_range(
        source,
        element.open_end,
        first_child,
        &current_selector,
        threshold,
        chunks,
        removals,
    );

    for (child_position, child_index) in element.children.iter().enumerate() {
        let child = &nodes[*child_index];
        let raw_len = child.end.saturating_sub(child.start);
        if raw_len > threshold && child.aid.is_some() {
            step_into_element(child, nodes, source, threshold, chunks, removals);
            current_selector = selector("S", child.aid.as_deref().unwrap_or(""));
        } else {
            emit_fragment(
                source,
                child.start,
                child.end,
                current_selector.clone(),
                chunks,
                removals,
            );
        }

        let tail_end = element
            .children
            .get(child_position + 1)
            .map(|index| nodes[*index].start)
            .unwrap_or(element.close_start);
        emit_text_range(
            source,
            child.end,
            tail_end,
            &current_selector,
            threshold,
            chunks,
            removals,
        );
    }

    if chunks.len() > first_chunk {
        chunks[first_chunk].starts_tags.push(aid.to_owned());
        chunks
            .last_mut()
            .expect("a chunk was just created")
            .ends_tags
            .push(aid.to_owned());
    }
}

fn emit_text_range(
    source: &[u8],
    start: usize,
    end: usize,
    selector: &str,
    threshold: usize,
    chunks: &mut Vec<RawFragment>,
    removals: &mut Vec<(usize, usize)>,
) {
    // Whitespace between sibling elements belongs to the same source range.
    // Keeping it in the payload makes adjacent DOM nodes merge up to the
    // target size; leaving every indentation run in SKEL would manufacture a
    // fragment for each pretty-printed element and can overflow FRAG's
    // u16-addressed detail record on large books.
    if start >= end {
        return;
    }
    removals.push((start, end));
    let mut cursor = start;
    while cursor < end {
        let mut boundary = (cursor + threshold).min(end);
        let text =
            std::str::from_utf8(&source[cursor..end]).expect("generated XHTML must be valid UTF-8");
        if boundary < end && !text.is_char_boundary(boundary - cursor) {
            while boundary > cursor && !text.is_char_boundary(boundary - cursor) {
                boundary -= 1;
            }
        }
        if boundary == cursor {
            boundary = (cursor + 1..=end)
                .find(|candidate| text.is_char_boundary(candidate - cursor))
                .unwrap_or(end);
        }
        emit_fragment(
            source,
            cursor,
            boundary,
            selector.to_owned(),
            chunks,
            &mut Vec::new(),
        );
        cursor = boundary;
    }
}

fn emit_fragment(
    source: &[u8],
    start: usize,
    end: usize,
    selector: String,
    chunks: &mut Vec<RawFragment>,
    removals: &mut Vec<(usize, usize)>,
) {
    if start >= end {
        return;
    }
    removals.push((start, end));
    chunks.push(RawFragment {
        raw: source[start..end].to_vec(),
        selector,
        source_start: start,
        source_end: end,
        starts_tags: Vec::new(),
        ends_tags: Vec::new(),
    });
}

fn merge_fragment_chunks(chunks: &mut Vec<RawFragment>, threshold: usize) {
    let mut merged = Vec::with_capacity(chunks.len());
    for chunk in chunks.drain(..) {
        let can_merge = merged.last().is_some_and(|previous: &RawFragment| {
            previous.ends_tags.is_empty()
                && chunk.starts_tags.is_empty()
                && previous.source_end == chunk.source_start
                && previous.raw.len() + chunk.raw.len() <= threshold
        });
        if can_merge {
            let previous = merged.last_mut().expect("merge candidate exists");
            previous.raw.extend_from_slice(&chunk.raw);
            previous.source_end = chunk.source_end;
            previous.ends_tags = chunk.ends_tags;
        } else {
            merged.push(chunk);
        }
    }
    *chunks = merged;
}

pub(crate) fn parse_elements(source: &[u8]) -> Vec<Element> {
    let mut nodes: Vec<Element> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut cursor = 0;
    while cursor < source.len() {
        let Some(relative) = source[cursor..].iter().position(|byte| *byte == b'<') else {
            break;
        };
        let start = cursor + relative;
        if let Some(next) = non_element_markup_end(source, start) {
            cursor = next;
            continue;
        }
        let Some(end) = tag_end(source, start) else {
            break;
        };
        if source.get(start + 1) == Some(&b'/') {
            let name = tag_name(&source[start..=end]);
            if let Some(stack_index) = stack
                .iter()
                .rposition(|index| nodes[*index].name.eq_ignore_ascii_case(&name))
            {
                while stack.len() > stack_index + 1 {
                    let index = stack.pop().expect("stack is non-empty");
                    nodes[index].close_start = nodes[index].open_end;
                    nodes[index].end = nodes[index].open_end;
                }
                let index = stack.pop().expect("matching element is on the stack");
                nodes[index].close_start = start;
                nodes[index].end = end + 1;
            }
        } else if source
            .get(start + 1)
            .is_some_and(|byte| !matches!(byte, b'!' | b'?'))
        {
            let tag = &source[start..=end];
            let name = tag_name(tag);
            if !name.is_empty() {
                let parent = stack.last().copied();
                let index = nodes.len();
                nodes.push(Element {
                    start,
                    open_end: end + 1,
                    close_start: end + 1,
                    end: end + 1,
                    name: name.clone(),
                    aid: attribute_value(tag, "aid"),
                    parent,
                    children: Vec::new(),
                });
                if let Some(parent) = parent {
                    nodes[parent].children.push(index);
                }
                if !tag.ends_with(b"/>") && !is_void_element(&name) {
                    if is_raw_text_name(&name) {
                        if let Some((close_start, close_end)) =
                            raw_text_element_end(source, end + 1, &name)
                        {
                            nodes[index].close_start = close_start;
                            nodes[index].end = close_end;
                            cursor = close_end;
                            continue;
                        }
                        // Generated XHTML is expected to close raw-text
                        // elements. If malformed input does not, consume the
                        // remainder as raw text rather than inventing nested
                        // elements from markup-like bytes in the payload.
                        nodes[index].close_start = source.len();
                        nodes[index].end = source.len();
                        cursor = source.len();
                        continue;
                    }
                    stack.push(index);
                }
            }
        }
        cursor = end + 1;
    }
    nodes
}

/// Return the exclusive end of markup that cannot contain ordinary elements.
/// Comments, CDATA, declarations, and processing instructions may contain
/// `<...>`-shaped bytes; treating those bytes as DOM nodes corrupts the
/// SKEL/FRAG parent context used by selectors.
pub(super) fn non_element_markup_end(source: &[u8], start: usize) -> Option<usize> {
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
        return tag_end(source, start).map(|end| end + 1);
    }
    None
}

pub(super) fn raw_text_element_end(
    source: &[u8],
    start: usize,
    name: &str,
) -> Option<(usize, usize)> {
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
                let end = tag_end(source, cursor)?;
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

fn tag_name(tag: &[u8]) -> String {
    let mut cursor = 1;
    while cursor < tag.len() && matches!(tag[cursor], b'/' | b'!' | b'?') {
        cursor += 1;
    }
    let start = cursor;
    while cursor < tag.len()
        && !tag[cursor].is_ascii_whitespace()
        && !matches!(tag[cursor], b'/' | b'>')
    {
        cursor += 1;
    }
    String::from_utf8_lossy(&tag[start..cursor]).into_owned()
}

fn attribute_value(tag: &[u8], name: &str) -> Option<String> {
    let name = name.as_bytes();
    let mut cursor = 1;
    while cursor < tag.len() {
        while cursor < tag.len() && tag[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= tag.len() || matches!(tag[cursor], b'/' | b'>') {
            break;
        }
        let start = cursor;
        while cursor < tag.len()
            && !tag[cursor].is_ascii_whitespace()
            && !matches!(tag[cursor], b'=' | b'/' | b'>')
        {
            cursor += 1;
        }
        let attribute = &tag[start..cursor];
        while cursor < tag.len() && tag[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= tag.len() || tag[cursor] != b'=' {
            continue;
        }
        cursor += 1;
        while cursor < tag.len() && tag[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let quote = *tag.get(cursor)?;
        if !matches!(quote, b'"' | b'\'') {
            return None;
        }
        let value_start = cursor + 1;
        let value_end = value_start
            + tag
                .get(value_start..)?
                .iter()
                .position(|byte| *byte == quote)?;
        if attribute.eq_ignore_ascii_case(name) {
            return String::from_utf8(tag[value_start..value_end].to_vec()).ok();
        }
        cursor = value_end + 1;
    }
    None
}

fn is_void_element(name: &str) -> bool {
    [
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
        "source", "track", "wbr",
    ]
    .iter()
    .any(|wanted| name.eq_ignore_ascii_case(wanted))
}

fn is_raw_text_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("script") || name.eq_ignore_ascii_case("style")
}

pub(crate) fn body_range(source: &str) -> Option<(usize, usize, usize)> {
    let bytes = source.as_bytes();
    let mut cursor = 0;
    let mut body = None;
    while let Some(relative) = bytes[cursor..].iter().position(|byte| *byte == b'<') {
        let start = cursor + relative;
        if let Some(next) = non_element_markup_end(bytes, start) {
            cursor = next;
            continue;
        }
        let end = tag_end(bytes, start)?;
        let (name_start, name_end, closing) = body_tag_name_range(bytes, start, end)?;
        if !closing && bytes[start..=end].ends_with(b"/>") {
            cursor = end + 1;
            continue;
        }
        if !closing && bytes[name_start..name_end].eq_ignore_ascii_case(b"body") {
            body = Some((start, end + 1));
        } else if closing
            && body.is_some_and(|(_, body_content_start)| body_content_start <= start)
            && bytes[name_start..name_end].eq_ignore_ascii_case(b"body")
        {
            let (body_start, body_content_start) = body.expect("body was checked above");
            return Some((body_start, body_content_start, start));
        }
        if !closing
            && (bytes[name_start..name_end].eq_ignore_ascii_case(b"script")
                || bytes[name_start..name_end].eq_ignore_ascii_case(b"style"))
            && !bytes[start..=end].ends_with(b"/>")
        {
            cursor = raw_text_element_end(bytes, end + 1, source.get(name_start..name_end)?)
                .map(|(_, close_end)| close_end)
                .unwrap_or(bytes.len());
        } else {
            cursor = end + 1;
        }
    }
    // A body without a closing tag is malformed and must not absorb the rest
    // of the document as body content.
    None
}

fn body_tag_name_range(bytes: &[u8], start: usize, end: usize) -> Option<(usize, usize, bool)> {
    let mut cursor = start + 1;
    let closing = bytes.get(cursor) == Some(&b'/');
    if closing {
        cursor += 1;
    }
    let name_start = cursor;
    while cursor < end
        && !bytes[cursor].is_ascii_whitespace()
        && !matches!(bytes[cursor], b'/' | b'>')
    {
        cursor += 1;
    }
    (name_start < cursor).then_some((name_start, cursor, closing))
}

pub(super) fn tag_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start + 1;
    let mut quote = None;
    while cursor < bytes.len() {
        match quote {
            Some(value) if bytes[cursor] == value => quote = None,
            None if bytes[cursor] == b'"' || bytes[cursor] == b'\'' => quote = Some(bytes[cursor]),
            None if bytes[cursor] == b'>' => return Some(cursor),
            _ => {}
        }
        cursor += 1;
    }
    None
}
