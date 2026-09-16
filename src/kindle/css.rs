/// Project the supported EPUB CSS subset based on KindleGen reference output. This is a
/// semantic stage: it only changes declarations in parsed CSS blocks and does
/// not resolve resources, imports, URLs, or stylesheet scope.
pub fn project_css_for_kindle(source: &str) -> String {
    project_css_range(source, 0, source.len())
}

/// Project declarations from an inline `style="..."` attribute with the
/// same declaration-level rules used for stylesheet blocks.
pub(crate) fn project_inline_style_for_kindle(source: &str) -> String {
    project_declarations(source)
}

fn project_css_range(source: &str, start: usize, end: usize) -> String {
    let Some(open) = find_top_level_open(source, start, end) else {
        return source[start..end].to_owned();
    };
    let Some(close) = matching_brace(source, open, end) else {
        return source[start..end].to_owned();
    };
    let prelude = &source[start..open];
    let body = &source[open + 1..close];
    let mut result = String::with_capacity(end - start);
    result.push_str(prelude);
    result.push('{');
    // A stylesheet may begin with statement at-rules such as @charset and
    // @namespace before its first style rule. Only the portion after the
    // final statement separator belongs to the block prelude; otherwise the
    // leading @charset would make every following declaration look nested
    // and silently skip projection.
    let block_prelude = prelude
        .rsplit_once(';')
        .map_or(prelude, |(_, remainder)| remainder);
    if block_prelude.trim_start().starts_with('@')
        || find_top_level_open(source, open + 1, close).is_some()
    {
        result.push_str(&project_css_range(source, open + 1, close));
    } else {
        result.push_str(&project_declarations(body));
    }
    result.push('}');
    result.push_str(&project_css_range(source, close + 1, end));
    result
}

fn project_declarations(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let mut segment_start = 0;
    let mut cursor = 0;
    let mut paren_depth = 0usize;
    while cursor < source.len() {
        let next = advance_css_char(source, cursor);
        match source.as_bytes()[cursor] {
            b'/' if source.as_bytes().get(cursor + 1) == Some(&b'*') => {
                cursor = skip_comment(source, cursor).unwrap_or(source.len());
            }
            b'\'' | b'"' => {
                cursor = skip_string(source, cursor).unwrap_or(source.len());
            }
            b'(' => {
                paren_depth += 1;
                cursor = next;
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                cursor = next;
            }
            b';' if paren_depth == 0 => {
                if append_declaration(&mut result, &source[segment_start..cursor]) {
                    result.push(';');
                }
                cursor = next;
                segment_start = cursor;
            }
            _ => cursor = next,
        }
    }
    append_declaration(&mut result, &source[segment_start..]);
    result
}

fn append_declaration(result: &mut String, declaration: &str) -> bool {
    let property_start = skip_css_trivia(declaration, 0);
    let property_end = find_property_colon(declaration, property_start).unwrap_or(property_start);
    if property_start == property_end {
        result.push_str(declaration);
        return true;
    }
    let property = remove_css_comments(&declaration[property_start..property_end])
        .trim()
        .to_ascii_lowercase();
    if matches!(property.as_str(), "max-width" | "max-height") {
        // Keep indentation/comments around the removed declaration so this
        // projection does not become a whitespace or comment normalizer.
        result.push_str(&declaration[..property_start]);
        return false;
    }
    let projected = match property.as_str() {
        "-epub-writing-mode" => Some("-webkit-writing-mode"),
        "-epub-text-combine" => Some("-webkit-text-combine"),
        value if value.starts_with("-epub-text-emphasis-") => Some("-webkit-text-emphasis-"),
        _ => None,
    };
    let Some(projected) = projected else {
        result.push_str(declaration);
        return true;
    };
    result.push_str(&declaration[..property_start]);
    if projected == "-webkit-text-emphasis-" {
        result.push_str(projected);
        result.push_str(&property["-epub-text-emphasis-".len()..]);
    } else {
        result.push_str(projected);
    }
    result.push_str(&declaration[property_end..]);
    true
}

fn find_top_level_open(source: &str, start: usize, end: usize) -> Option<usize> {
    let mut cursor = start;
    while cursor < end {
        match source.as_bytes()[cursor] {
            b'/' if source.as_bytes().get(cursor + 1) == Some(&b'*') => {
                cursor = skip_comment(source, cursor).unwrap_or(end)
            }
            b'\'' | b'"' => cursor = skip_string(source, cursor).unwrap_or(end),
            b'{' => return Some(cursor),
            _ => cursor = advance_css_char(source, cursor),
        }
    }
    None
}

fn matching_brace(source: &str, open: usize, end: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut cursor = open + 1;
    while cursor < end {
        match source.as_bytes()[cursor] {
            b'/' if source.as_bytes().get(cursor + 1) == Some(&b'*') => {
                cursor = skip_comment(source, cursor).unwrap_or(end)
            }
            b'\'' | b'"' => cursor = skip_string(source, cursor).unwrap_or(end),
            b'{' => {
                depth += 1;
                cursor = advance_css_char(source, cursor);
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(cursor);
                }
                cursor = advance_css_char(source, cursor);
            }
            _ => cursor = advance_css_char(source, cursor),
        }
    }
    None
}

fn skip_comment(source: &str, start: usize) -> Option<usize> {
    source[start + 2..]
        .find("*/")
        .map(|offset| start + 2 + offset + 2)
}

fn skip_string(source: &str, start: usize) -> Option<usize> {
    let quote = *source.as_bytes().get(start)?;
    let mut cursor = start + 1;
    while cursor < source.len() {
        match source.as_bytes()[cursor] {
            byte if byte == quote => return Some(cursor + 1),
            b'\\' => {
                cursor = advance_css_char(source, cursor);
                cursor = advance_css_char(source, cursor);
            }
            _ => cursor = advance_css_char(source, cursor),
        }
    }
    None
}

fn skip_css_trivia(source: &str, mut cursor: usize) -> usize {
    loop {
        while source
            .as_bytes()
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            cursor += 1;
        }
        if source.as_bytes().get(cursor) == Some(&b'/')
            && source.as_bytes().get(cursor + 1) == Some(&b'*')
        {
            cursor = skip_comment(source, cursor).unwrap_or(source.len());
            continue;
        }
        return cursor;
    }
}

fn find_property_colon(source: &str, start: usize) -> Option<usize> {
    let mut cursor = start;
    while cursor < source.len() {
        match source.as_bytes()[cursor] {
            b'/' if source.as_bytes().get(cursor + 1) == Some(&b'*') => {
                cursor = skip_comment(source, cursor).unwrap_or(source.len());
            }
            b'\'' | b'"' => {
                cursor = skip_string(source, cursor).unwrap_or(source.len());
            }
            b':' => return Some(cursor),
            _ => cursor = advance_css_char(source, cursor),
        }
    }
    None
}

fn remove_css_comments(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let mut cursor = 0;
    while cursor < source.len() {
        if source.as_bytes()[cursor] == b'/' && source.as_bytes().get(cursor + 1) == Some(&b'*') {
            cursor = skip_comment(source, cursor).unwrap_or(source.len());
        } else {
            let next = advance_css_char(source, cursor);
            result.push_str(&source[cursor..next]);
            cursor = next;
        }
    }
    result
}

fn advance_css_char(source: &str, cursor: usize) -> usize {
    source
        .get(cursor..)
        .and_then(|remaining| remaining.chars().next())
        .map_or(source.len(), |character| cursor + character.len_utf8())
}
