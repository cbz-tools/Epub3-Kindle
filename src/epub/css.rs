use std::collections::{HashMap, HashSet};

use crate::book::{ContentDocument, CssDeclaration, CssRule, Resource, StyleSheet};
use crate::css::{css_import_spans, css_import_targets, is_remote_reference};
use crate::error::{Error, Result};
use crate::xhtml::path::{
    is_external_reference, normalize_path_lossy as normalize_path, resolve_path,
};
use crate::{WarningCode, WarningCollector};

pub fn parse_css(href: impl Into<String>, source: impl Into<String>) -> StyleSheet {
    let href = href.into();
    let source = source.into();
    let without_comments = remove_comments_legacy(&source);
    let mut rules = Vec::new();
    for part in without_comments.split('}') {
        let Some((selector, declarations)) = part.split_once('{') else {
            continue;
        };
        let selector = selector.trim();
        if selector.is_empty() {
            continue;
        }
        let declarations = declarations
            .split(';')
            .filter_map(|declaration| {
                let (property, value) = declaration.split_once(':')?;
                let property = property.trim().to_ascii_lowercase();
                let value = value.trim().to_owned();
                (!property.is_empty() && !value.is_empty())
                    .then_some(CssDeclaration { property, value })
            })
            .collect();
        rules.push(CssRule {
            selector: selector.to_owned(),
            declarations,
        });
    }
    StyleSheet {
        href,
        source,
        rules,
    }
}

pub(super) fn active_css_stylesheets(
    index: &CssResourceIndex<'_>,
    content: &[ContentDocument],
    warnings: &mut WarningCollector,
) -> Result<HashSet<String>> {
    let roots = content
        .iter()
        .flat_map(|document| {
            document
                .referenced_styles
                .iter()
                .map(|reference| (document.href.clone(), reference.clone()))
        })
        .collect::<Vec<_>>();
    let mut active = HashSet::new();
    let mut pending = roots;
    while let Some((base_href, reference)) = pending.pop() {
        let Some(resolved) = resolve_path(&base_href, &reference) else {
            if !is_external_reference(&reference) {
                return Err(Error::InvalidEpub(format!(
                    "CSS resource path {reference} escapes the EPUB root"
                )));
            }
            if is_remote_reference(&reference) {
                warnings.add_once(
                    WarningCode::W004,
                    "remote CSS @import or stylesheet reference was dropped without fetching",
                );
            }
            continue;
        };
        if !active.insert(resolved.clone()) {
            continue;
        }
        let Some(resource) = index.by_href.get(&resolved).copied() else {
            continue;
        };
        let source = std::str::from_utf8(&resource.data)
            .expect("EPUB CSS resources are normalized to UTF-8");
        let import_base_href = index.import_base_href(resource);
        pending.extend(
            css_import_targets(source)
                .into_iter()
                .map(|target| (import_base_href.clone(), target)),
        );
    }
    Ok(active)
}

/// Return local URLs found in `@font-face` blocks. The caller resolves these
/// URLs against the stylesheet and applies font-specific policy only to the
/// referenced resources.
pub(super) fn font_face_resource_references(source: &str) -> Result<Vec<String>> {
    let mut references = Vec::new();
    scan_font_face_blocks(source, 0, source.len(), &mut references)?;
    Ok(references)
}

fn scan_font_face_blocks(
    source: &str,
    start: usize,
    end: usize,
    references: &mut Vec<String>,
) -> Result<()> {
    let mut cursor = start;
    while cursor < end {
        cursor = skip_whitespace_and_comments(source, cursor, end)?;
        if cursor >= end {
            break;
        }
        let statement_start = cursor;
        let (boundary, kind) = find_boundary(source, cursor, end)?;
        match kind {
            Boundary::Semicolon => cursor = boundary + 1,
            Boundary::OpenBrace => {
                let close = matching_brace(source, boundary, end)?;
                if is_font_face_prelude(source, statement_start, boundary)? {
                    references.extend(css_url_targets(source, boundary + 1, close)?);
                } else if is_at_rule_prelude(source, statement_start, boundary)? {
                    // Grouping at-rules such as @media and @supports may
                    // contain a valid @font-face statement. Qualified rule
                    // blocks contain declarations, where the token must not
                    // be mistaken for another CSS statement.
                    scan_font_face_blocks(source, boundary + 1, close, references)?;
                }
                cursor = close + 1;
            }
            Boundary::CloseBrace => return malformed_css("unexpected closing brace"),
            Boundary::End => break,
        }
    }
    Ok(())
}

fn is_font_face_prelude(source: &str, start: usize, boundary: usize) -> Result<bool> {
    let token_end = start + "@font-face".len();
    if source
        .get(start..token_end)
        .is_none_or(|candidate| !candidate.eq_ignore_ascii_case("@font-face"))
    {
        return Ok(false);
    }
    let after_token = skip_whitespace_and_comments(source, token_end, boundary)?;
    Ok(after_token == boundary)
}

fn is_at_rule_prelude(source: &str, start: usize, boundary: usize) -> Result<bool> {
    let start = skip_whitespace_and_comments(source, start, boundary)?;
    Ok(source.as_bytes().get(start) == Some(&b'@'))
}

pub(super) fn validate_local_resource_paths(source: &str, stylesheet_href: &str) -> Result<()> {
    for target in css_url_targets(source, 0, source.len())? {
        if is_fragment_only_reference(&target) {
            continue;
        }
        if !is_external_reference(&target) && resolve_path(stylesheet_href, &target).is_none() {
            return Err(Error::InvalidEpub(format!(
                "CSS resource path {target} escapes the EPUB root"
            )));
        }
    }
    Ok(())
}

fn is_fragment_only_reference(target: &str) -> bool {
    target.starts_with('#')
}

fn css_url_targets(source: &str, start: usize, end: usize) -> Result<Vec<String>> {
    let mut references = Vec::new();
    let mut cursor = start;
    while cursor < end {
        if let Some(next) = skip_url(source, cursor, end)? {
            if let Some(target) = css_url_target(source, cursor, next) {
                references.push(target);
            }
            cursor = next;
        } else if source.as_bytes()[cursor] == b'/'
            && source.as_bytes().get(cursor + 1) == Some(&b'*')
        {
            cursor = skip_comment(source, cursor, end)?;
        } else if matches!(source.as_bytes()[cursor], b'\'' | b'"') {
            cursor = skip_string(source, cursor, end)?;
        } else {
            cursor = advance_char(source, cursor);
        }
    }
    Ok(references)
}

fn css_url_target(source: &str, start: usize, end: usize) -> Option<String> {
    let open = source[start..end].find('(')? + start + 1;
    let mut cursor = open;
    while cursor < end && source.as_bytes()[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    if source
        .as_bytes()
        .get(cursor)
        .is_some_and(|byte| *byte == b'\'' || *byte == b'"')
    {
        let quote = source.as_bytes()[cursor];
        let value_start = cursor + 1;
        let value_end = source[value_start..end]
            .find(quote as char)
            .map(|offset| value_start + offset)?;
        return Some(source[value_start..value_end].to_owned());
    }
    let value_start = cursor;
    while cursor < end && !source.as_bytes()[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    Some(source[value_start..cursor].trim_end_matches(')').to_owned())
}

fn advance_char(source: &str, cursor: usize) -> usize {
    source[cursor..]
        .chars()
        .next()
        .map_or(source.len(), |character| cursor + character.len_utf8())
}

pub(super) struct CssResourceIndex<'a> {
    by_href: HashMap<String, &'a Resource>,
    synthetic_origins: HashMap<String, &'a str>,
}

impl<'a> CssResourceIndex<'a> {
    pub(super) fn new(content: &'a [ContentDocument], resources: &'a [Resource]) -> Self {
        let mut by_href = HashMap::new();
        let mut synthetic_hrefs = HashSet::new();
        for resource in resources {
            if !resource.media_type.eq_ignore_ascii_case("text/css") {
                continue;
            }
            let href = normalize_path(&resource.href);
            by_href.entry(href.clone()).or_insert(resource);
            if resource
                .properties
                .iter()
                .any(|property| property == crate::css::SYNTHETIC_INLINE_CSS_PROPERTY)
            {
                synthetic_hrefs.insert(href);
            }
        }

        let mut synthetic_origins = HashMap::new();
        if !synthetic_hrefs.is_empty() {
            for document in content {
                for reference in &document.referenced_styles {
                    let Some(resolved) = resolve_path(&document.href, reference) else {
                        continue;
                    };
                    if synthetic_hrefs.contains(&resolved) {
                        synthetic_origins
                            .entry(resolved)
                            .or_insert(document.href.as_str());
                    }
                }
            }
        }

        Self {
            by_href,
            synthetic_origins,
        }
    }

    pub(super) fn import_base_href(&self, resource: &Resource) -> String {
        let is_synthetic = resource
            .properties
            .iter()
            .any(|property| property == crate::css::SYNTHETIC_INLINE_CSS_PROPERTY);
        if !is_synthetic {
            return resource.href.clone();
        }
        let resource_href = normalize_path(&resource.href);
        self.synthetic_origins
            .get(&resource_href)
            .copied()
            .map(str::to_owned)
            .unwrap_or_else(|| resource.href.clone())
    }
}

pub(super) fn css_resource_base_href(index: &CssResourceIndex<'_>, resource: &Resource) -> String {
    index.import_base_href(resource)
}

pub(super) fn validate_kf8_css(source: &str) -> Result<()> {
    scan_stylesheet(source, 0, source.len())
}

pub(super) fn validate_kf8_css_with_warnings(
    source: &str,
    warnings: &mut WarningCollector,
) -> Result<()> {
    match validate_kf8_css(source) {
        Ok(()) => Ok(()),
        Err(error @ Error::UnsupportedEpub(_)) => {
            warnings.add_once(
                WarningCode::W004,
                format!("CSS presentation semantics were degraded: {error}"),
            );
            Ok(())
        }
        Err(error) => Err(error),
    }
}

pub(super) fn validate_kf8_inline_style(source: &str) -> Result<()> {
    scan_inline_declarations(source)
}

pub(super) fn validate_kf8_inline_style_with_warnings(
    source: &str,
    warnings: &mut WarningCollector,
) -> Result<()> {
    match validate_kf8_inline_style(source) {
        Ok(()) => Ok(()),
        Err(error @ Error::UnsupportedEpub(_)) => {
            warnings.add_once(
                WarningCode::W004,
                format!("inline CSS presentation semantics were degraded: {error}"),
            );
            Ok(())
        }
        Err(error) => Err(error),
    }
}

pub(super) fn warn_remote_css_references(
    source: &str,
    warnings: &mut WarningCollector,
) -> Result<()> {
    let has_remote_import = css_import_spans(source)
        .iter()
        .any(|span| is_remote_reference(&source[span.target_start..span.target_end]));
    let has_remote_url = css_url_targets(source, 0, source.len())?
        .iter()
        .any(|target| is_remote_reference(target));
    if has_remote_import || has_remote_url {
        warnings.add_once(
            WarningCode::W004,
            "remote CSS url() references were dropped without fetching",
        );
    }
    Ok(())
}

fn scan_stylesheet(source: &str, start: usize, end: usize) -> Result<()> {
    let mut cursor = start;
    while cursor < end {
        cursor = skip_whitespace_and_comments(source, cursor, end)?;
        if cursor >= end {
            break;
        }
        if source.as_bytes()[cursor] == b'}' {
            return malformed_css("unexpected closing brace");
        }
        let statement_start = cursor;
        let (boundary, kind) = find_boundary(source, cursor, end)?;
        match kind {
            Boundary::Semicolon => cursor = boundary + 1,
            Boundary::OpenBrace => {
                let close = matching_brace(source, boundary, end)?;
                let prelude = &source[statement_start..boundary];
                if !prelude.trim_start().starts_with('@') {
                    validate_selector(prelude)?;
                }
                scan_mixed_block(source, boundary + 1, close)?;
                cursor = close + 1;
            }
            Boundary::CloseBrace => return malformed_css("unexpected closing brace"),
            Boundary::End => {
                if !source[statement_start..end].trim().is_empty() {
                    return malformed_css("unterminated CSS statement");
                }
                cursor = end;
            }
        }
    }
    Ok(())
}

fn scan_mixed_block(source: &str, start: usize, end: usize) -> Result<()> {
    let mut cursor = start;
    while cursor < end {
        cursor = skip_whitespace_and_comments(source, cursor, end)?;
        if cursor >= end {
            break;
        }
        let statement_start = cursor;
        let (boundary, kind) = find_boundary(source, cursor, end)?;
        match kind {
            Boundary::Semicolon => {
                validate_declaration(&source[statement_start..boundary], false)?;
                cursor = boundary + 1;
            }
            Boundary::OpenBrace => {
                let close = matching_brace(source, boundary, end)?;
                let prelude = &source[statement_start..boundary];
                if !prelude.trim_start().starts_with('@') {
                    validate_selector(prelude)?;
                }
                scan_mixed_block(source, boundary + 1, close)?;
                cursor = close + 1;
            }
            Boundary::CloseBrace => return malformed_css("unexpected closing brace"),
            Boundary::End => {
                validate_declaration(&source[statement_start..end], false)?;
                cursor = end;
            }
        }
    }
    Ok(())
}

fn scan_inline_declarations(source: &str) -> Result<()> {
    let mut cursor = 0;
    let mut statement_start = 0;
    while cursor < source.len() {
        cursor = skip_whitespace_and_comments(source, cursor, source.len())?;
        if cursor >= source.len() {
            statement_start = cursor;
            break;
        }
        let (boundary, kind) = find_boundary(source, cursor, source.len())?;
        match kind {
            Boundary::Semicolon => {
                validate_declaration(&source[statement_start..boundary], true)?;
                cursor = boundary + 1;
                statement_start = cursor;
            }
            Boundary::OpenBrace | Boundary::CloseBrace => {
                return malformed_css("inline style contains a CSS block");
            }
            Boundary::End => {
                validate_declaration(&source[statement_start..source.len()], true)?;
                cursor = source.len();
            }
        }
    }
    if statement_start < source.len() && !source[statement_start..].trim().is_empty() {
        validate_declaration(&source[statement_start..], true)?;
    }
    Ok(())
}

fn validate_selector(selector: &str) -> Result<()> {
    let bytes = selector.as_bytes();
    let mut cursor = 0;
    let mut bracket_depth = 0usize;
    let mut nth_function_open = None;
    let mut nth_function_depth = 0usize;
    while cursor < bytes.len() {
        if bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
            continue;
        }
        if bytes[cursor] == b'/' && bytes.get(cursor + 1) == Some(&b'*') {
            cursor = skip_comment(selector, cursor, bytes.len())?;
            continue;
        }
        if matches!(bytes[cursor], b'\'' | b'"') {
            cursor = skip_string(selector, cursor, bytes.len())?;
            continue;
        }
        if let Some(next) = skip_url(selector, cursor, bytes.len())? {
            cursor = next;
            continue;
        }
        if bytes[cursor] == b'\\' {
            cursor = skip_escape(selector, cursor);
            continue;
        }
        match bytes[cursor] {
            b'[' => bracket_depth += 1,
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b'(' if nth_function_open == Some(cursor) => {
                nth_function_open = None;
                nth_function_depth = 1;
            }
            b'(' if nth_function_depth > 0 => nth_function_depth += 1,
            b')' if nth_function_depth > 0 => nth_function_depth -= 1,
            b'+' | b'~' if bracket_depth == 0 && nth_function_depth == 0 => {
                return Err(unsupported_css(
                    "sibling combinators (+ and ~) are unsupported by the KF8 projection",
                ));
            }
            b':' if bracket_depth == 0 => {
                let mut name_end = cursor + 1;
                if bytes.get(name_end) == Some(&b':') {
                    name_end += 1;
                }
                let name_start = name_end;
                while name_end < bytes.len() && is_ident_byte(bytes[name_end]) {
                    name_end += 1;
                }
                let name = selector[name_start..name_end].to_ascii_lowercase();
                let function_open = skip_whitespace_and_comments(selector, name_end, bytes.len())?;
                let is_nth_function = matches!(
                    name.as_str(),
                    "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type"
                ) && bytes.get(function_open) == Some(&b'(');
                if is_nth_function {
                    nth_function_open = Some(function_open);
                }
                let nth_child_function = name == "nth-child" && is_nth_function;
                if matches!(
                    name.as_str(),
                    "before" | "after" | "first-letter" | "first-line"
                ) {
                    return Err(unsupported_css(
                        "pseudo-elements (::before, ::after, ::first-letter, and ::first-line) are unsupported by the KF8 projection",
                    ));
                }
                if name == "first-child" || nth_child_function {
                    return Err(unsupported_css(
                        ":first-child and :nth-child() pseudo-classes are unsupported by the KF8 projection",
                    ));
                }
                cursor = name_end;
                continue;
            }
            _ => {}
        }
        cursor += 1;
    }
    if bracket_depth != 0 {
        return malformed_css("unterminated selector attribute selector");
    }
    Ok(())
}

fn validate_declaration(declaration: &str, strict: bool) -> Result<()> {
    let declaration = declaration.trim();
    if declaration.is_empty() {
        return Ok(());
    }
    let Some(colon) = top_level_colon(declaration)? else {
        if strict {
            return malformed_css("inline style declaration has no property separator");
        }
        return Ok(());
    };
    let property = remove_comments(&declaration[..colon])?
        .trim()
        .to_ascii_lowercase();
    if property.is_empty() {
        return if strict {
            malformed_css("inline style declaration has an empty property")
        } else {
            Ok(())
        };
    }
    if property.starts_with("--") {
        return Ok(());
    }
    let value = &declaration[colon + 1..];
    if property == "counter-reset" || property == "counter-increment" {
        return Err(unsupported_css(&format!(
            "the {property} CSS declaration is unsupported by the KF8 projection"
        )));
    }
    if property == "content" && !is_safe_content_value(value)? {
        return Err(unsupported_css(
            "generated content declarations are unsupported by the KF8 projection",
        ));
    }
    if contains_counter_function(value)? {
        return Err(unsupported_css(
            "counter() and counters() CSS functions are unsupported by the KF8 projection",
        ));
    }
    Ok(())
}

fn is_safe_content_value(value: &str) -> Result<bool> {
    let value = remove_comments(value)?;
    let tokens = value.split_whitespace().collect::<Vec<_>>();
    Ok(matches!(
        tokens.as_slice(),
        [token] if token.eq_ignore_ascii_case("normal") || token.eq_ignore_ascii_case("none")
    ) || matches!(
        tokens.as_slice(),
        [token, important]
            if (token.eq_ignore_ascii_case("normal") || token.eq_ignore_ascii_case("none"))
                && important.eq_ignore_ascii_case("!important")
    ))
}

fn contains_counter_function(value: &str) -> Result<bool> {
    let bytes = value.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
            continue;
        }
        if bytes[cursor] == b'/' && bytes.get(cursor + 1) == Some(&b'*') {
            cursor = skip_comment(value, cursor, bytes.len())?;
            continue;
        }
        if matches!(bytes[cursor], b'\'' | b'"') {
            cursor = skip_string(value, cursor, bytes.len())?;
            continue;
        }
        if let Some(next) = skip_url(value, cursor, bytes.len())? {
            cursor = next;
            continue;
        }
        if bytes[cursor] == b'\\' {
            cursor = skip_escape(value, cursor);
            continue;
        }
        if is_ident_start(bytes[cursor]) {
            let start = cursor;
            cursor += 1;
            while cursor < bytes.len() && is_ident_byte(bytes[cursor]) {
                cursor += 1;
            }
            let name = &value[start..cursor];
            let after_name = skip_whitespace_and_comments(value, cursor, bytes.len())?;
            if (name.eq_ignore_ascii_case("counter") || name.eq_ignore_ascii_case("counters"))
                && bytes.get(after_name) == Some(&b'(')
            {
                return Ok(true);
            }
            cursor = after_name;
            continue;
        }
        cursor += 1;
    }
    Ok(false)
}

fn top_level_colon(source: &str) -> Result<Option<usize>> {
    let bytes = source.as_bytes();
    let mut cursor = 0;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    while cursor < bytes.len() {
        if bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
            continue;
        }
        if bytes[cursor] == b'/' && bytes.get(cursor + 1) == Some(&b'*') {
            cursor = skip_comment(source, cursor, bytes.len())?;
            continue;
        }
        if matches!(bytes[cursor], b'\'' | b'"') {
            cursor = skip_string(source, cursor, bytes.len())?;
            continue;
        }
        if let Some(next) = skip_url(source, cursor, bytes.len())? {
            cursor = next;
            continue;
        }
        if bytes[cursor] == b'\\' {
            cursor = skip_escape(source, cursor);
            continue;
        }
        match bytes[cursor] {
            b'[' => bracket_depth += 1,
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b'(' => paren_depth += 1,
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b':' if bracket_depth == 0 && paren_depth == 0 => return Ok(Some(cursor)),
            _ => {}
        }
        cursor += 1;
    }
    if bracket_depth != 0 || paren_depth != 0 {
        return malformed_css("unterminated declaration value grouping");
    }
    Ok(None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Boundary {
    Semicolon,
    OpenBrace,
    CloseBrace,
    End,
}

fn find_boundary(source: &str, start: usize, end: usize) -> Result<(usize, Boundary)> {
    let bytes = source.as_bytes();
    let mut cursor = start;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    while cursor < end {
        if bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
            continue;
        }
        if bytes[cursor] == b'/' && bytes.get(cursor + 1) == Some(&b'*') {
            cursor = skip_comment(source, cursor, end)?;
            continue;
        }
        if matches!(bytes[cursor], b'\'' | b'"') {
            cursor = skip_string(source, cursor, end)?;
            continue;
        }
        if let Some(next) = skip_url(source, cursor, end)? {
            cursor = next;
            continue;
        }
        if bytes[cursor] == b'\\' {
            cursor = skip_escape(source, cursor);
            continue;
        }
        match bytes[cursor] {
            b'[' => bracket_depth += 1,
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b'(' => paren_depth += 1,
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b';' if bracket_depth == 0 && paren_depth == 0 => {
                return Ok((cursor, Boundary::Semicolon));
            }
            b'{' if bracket_depth == 0 && paren_depth == 0 => {
                return Ok((cursor, Boundary::OpenBrace));
            }
            b'}' if bracket_depth == 0 && paren_depth == 0 => {
                return Ok((cursor, Boundary::CloseBrace));
            }
            _ => {}
        }
        cursor += 1;
    }
    if bracket_depth != 0 || paren_depth != 0 {
        return malformed_css("unterminated CSS grouping");
    }
    Ok((end, Boundary::End))
}

fn matching_brace(source: &str, open: usize, end: usize) -> Result<usize> {
    let bytes = source.as_bytes();
    let mut cursor = open + 1;
    let mut depth = 1usize;
    while cursor < end {
        if bytes[cursor] == b'/' && bytes.get(cursor + 1) == Some(&b'*') {
            cursor = skip_comment(source, cursor, end)?;
            continue;
        }
        if matches!(bytes[cursor], b'\'' | b'"') {
            cursor = skip_string(source, cursor, end)?;
            continue;
        }
        if let Some(next) = skip_url(source, cursor, end)? {
            cursor = next;
            continue;
        }
        if bytes[cursor] == b'\\' {
            cursor = skip_escape(source, cursor);
            continue;
        }
        match bytes[cursor] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(cursor);
                }
            }
            _ => {}
        }
        cursor += 1;
    }
    malformed_css("unterminated CSS block")
}

fn skip_whitespace_and_comments(source: &str, mut cursor: usize, end: usize) -> Result<usize> {
    let bytes = source.as_bytes();
    while cursor < end {
        if bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        } else if bytes[cursor] == b'/' && bytes.get(cursor + 1) == Some(&b'*') {
            cursor = skip_comment(source, cursor, end)?;
        } else {
            break;
        }
    }
    Ok(cursor)
}

fn skip_comment(source: &str, start: usize, end: usize) -> Result<usize> {
    let Some(relative_end) = source[start + 2..end].find("*/") else {
        return malformed_css("unterminated CSS comment");
    };
    Ok(start + 2 + relative_end + 2)
}

fn skip_string(source: &str, start: usize, end: usize) -> Result<usize> {
    let quote = source.as_bytes()[start];
    let bytes = source.as_bytes();
    let mut cursor = start + 1;
    while cursor < end {
        if bytes[cursor] == b'\\' {
            cursor = skip_escape(source, cursor);
        } else if bytes[cursor] == quote {
            return Ok(cursor + 1);
        } else {
            cursor += 1;
        }
    }
    malformed_css("unterminated CSS string")
}

fn skip_url(source: &str, start: usize, end: usize) -> Result<Option<usize>> {
    let bytes = source.as_bytes();
    if !is_ident_start(*bytes.get(start).unwrap_or(&0)) {
        return Ok(None);
    }
    let mut name_end = start + 1;
    while name_end < end && is_ident_byte(bytes[name_end]) {
        name_end += 1;
    }
    if !source[start..name_end].eq_ignore_ascii_case("url") {
        return Ok(None);
    }
    let open = skip_whitespace_and_comments(source, name_end, end)?;
    if bytes.get(open) != Some(&b'(') {
        return Ok(None);
    }
    Ok(Some(skip_parentheses(source, open, end)?))
}

fn skip_parentheses(source: &str, open: usize, end: usize) -> Result<usize> {
    let bytes = source.as_bytes();
    let mut cursor = open + 1;
    let mut depth = 1usize;
    while cursor < end {
        if bytes[cursor] == b'/' && bytes.get(cursor + 1) == Some(&b'*') {
            cursor = skip_comment(source, cursor, end)?;
            continue;
        }
        if matches!(bytes[cursor], b'\'' | b'"') {
            cursor = skip_string(source, cursor, end)?;
            continue;
        }
        if bytes[cursor] == b'\\' {
            cursor = skip_escape(source, cursor);
            continue;
        }
        match bytes[cursor] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(cursor + 1);
                }
            }
            _ => {}
        }
        cursor += 1;
    }
    malformed_css("unterminated CSS function")
}

fn skip_escape(source: &str, cursor: usize) -> usize {
    source
        .get(cursor..)
        .and_then(|remaining| remaining.chars().next())
        .map_or(source.len(), |character| {
            let next = cursor + character.len_utf8();
            source
                .get(next..)
                .and_then(|remaining| remaining.chars().next())
                .map_or(source.len(), |escaped| next + escaped.len_utf8())
        })
}

fn remove_comments(source: &str) -> Result<String> {
    let mut result = String::with_capacity(source.len());
    let mut cursor = 0;
    while cursor < source.len() {
        if source.as_bytes()[cursor] == b'/' && source.as_bytes().get(cursor + 1) == Some(&b'*') {
            cursor = skip_comment(source, cursor, source.len())?;
        } else {
            let next = if source.as_bytes()[cursor] == b'\\' {
                skip_escape(source, cursor)
            } else {
                source[cursor..]
                    .chars()
                    .next()
                    .map_or(source.len(), |character| cursor + character.len_utf8())
            };
            result.push_str(&source[cursor..next]);
            cursor = next;
        }
    }
    Ok(result)
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'-')
}

fn is_ident_byte(byte: u8) -> bool {
    is_ident_start(byte) || byte.is_ascii_digit()
}

fn malformed_css<T>(detail: &str) -> Result<T> {
    Err(Error::InvalidXhtmlCss(format!("malformed CSS: {detail}")))
}

fn unsupported_css(detail: &str) -> Error {
    Error::UnsupportedEpub(detail.to_owned())
}

fn remove_comments_legacy(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find("/*") {
        result.push_str(&rest[..start]);
        let Some(end) = rest[start + 2..].find("*/") else {
            break;
        };
        rest = &rest[start + 2 + end + 2..];
    }
    result.push_str(rest);
    result
}
