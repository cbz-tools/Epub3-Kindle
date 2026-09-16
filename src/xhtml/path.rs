//! Pure href and path syntax operations shared by EPUB and KF8 stages.
//!
//! Functions here normalize paths, resolve relative references, and identify
//! external references. EPUB semantic and KF8 flow decisions belong to callers.

pub(crate) fn resolve_path(base_href: &str, target: &str) -> Option<String> {
    let target = target.split(['#', '?']).next().unwrap_or(target);
    if target.is_empty() || is_external_reference(target) {
        return None;
    }
    let target = percent_decode(target)?;
    let base_href = percent_decode(base_href).unwrap_or_else(|| base_href.to_owned());
    let base = if target.starts_with('/') {
        target.to_owned()
    } else if let Some((directory, _)) = base_href.rsplit_once('/') {
        format!("{directory}/{target}")
    } else {
        target.to_owned()
    };
    normalize_decoded_path(&base)
}

pub(crate) fn normalize_path(path: &str) -> Option<String> {
    normalize_path_checked(path)
}

pub(crate) fn normalize_path_lossy(path: &str) -> String {
    normalize_path_checked(path).unwrap_or_default()
}

/// Normalize a package-relative path while retaining root-escape failures.
///
/// The lossy helper is retained for existing lookup/index callers, but input
/// validation must use this form so `../` cannot silently disappear.
pub(crate) fn normalize_path_checked(path: &str) -> Option<String> {
    // A literal percent is a valid OCF path character. Decode well-formed URI
    // escapes, while retaining the historical lookup behavior for malformed
    // escapes instead of turning it into an unrelated path rejection.
    let normalized = percent_decode(path).unwrap_or_else(|| path.to_owned());
    normalize_decoded_path(&normalized)
}

fn normalize_decoded_path(path: &str) -> Option<String> {
    let mut components = Vec::new();
    let normalized = path.replace('\\', "/");
    for component in normalized.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            value => components.push(value),
        }
    }
    (!components.is_empty()).then(|| components.join("/"))
}

/// Decode URI percent escapes used by EPUB hrefs without adding a dependency.
/// Literal UTF-8 remains unchanged; percent-encoded bytes must form UTF-8.
pub(crate) fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] == b'%' {
            let high = bytes.get(cursor + 1).and_then(|byte| hex_value(*byte))?;
            let low = bytes.get(cursor + 2).and_then(|byte| hex_value(*byte))?;
            decoded.push((high << 4) | low);
            cursor += 3;
        } else {
            let character = value[cursor..].chars().next()?;
            let mut encoded = [0; 4];
            decoded.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
            cursor += character.len_utf8();
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn is_external_reference(target: &str) -> bool {
    target.starts_with("data:")
        || target.starts_with("//")
        || target.contains("://")
        || target.starts_with("kindle:")
        || has_uri_scheme(target)
}

fn has_uri_scheme(target: &str) -> bool {
    let Some((scheme, _)) = target.split_once(':') else {
        return false;
    };
    !scheme.is_empty()
        && scheme.chars().enumerate().all(|(index, character)| {
            if index == 0 {
                character.is_ascii_alphabetic()
            } else {
                character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
            }
        })
}
