use std::collections::HashSet;
use std::io::Cursor;
use std::path::Path;

use super::super::navigation::{canonicalize_navigation, parse_nav_xhtml, parse_ncx};
use super::super::opf::has_property;
use super::super::package_archive::{BoundedZipArchive, read_zip_entry};
use super::manifest::ManifestIdIndex;
use super::resolve_href;
use crate::book::Navigation;
use crate::error::Result;

pub(super) fn load_navigation(
    archive: &mut BoundedZipArchive<Cursor<&[u8]>>,
    parsed: &super::super::opf::ParsedOpf,
    base: &Path,
    manifest_id_index: &ManifestIdIndex,
) -> Result<Navigation> {
    // Direct SVG spine items are exposed as generated XHTML wrapper sections.
    // Their source hrefs therefore need the same manifest-coordinate mapping
    // as XHTML documents during navigation canonicalization; other SVG assets
    // remain ordinary resources and are not position targets.
    let direct_svg_spine_ids = parsed
        .spine
        .iter()
        .filter_map(|spine_item| manifest_id_index.get(&parsed.manifest, &spine_item.idref))
        .filter(|item| item.media_type.eq_ignore_ascii_case("image/svg+xml"))
        .map(|item| item.id.clone())
        .collect::<HashSet<_>>();
    let nav_item = parsed
        .manifest
        .iter()
        .find(|item| has_property(item, "nav"));
    let nav_is_primary = parsed.ncx_id.is_none() && nav_item.is_some();
    let (mut navigation, navigation_path, supplemental_nav) =
        if let Some(ncx_id) = parsed.ncx_id.as_deref() {
            let item = manifest_id_index.get(&parsed.manifest, ncx_id);
            if let Some(item) = item {
                let path = resolve_href(base, &item.href);
                (
                    parse_ncx(&read_zip_entry(archive, &path)?)?,
                    Some(path),
                    nav_item.map(|nav_item| resolve_href(base, &nav_item.href)),
                )
            } else {
                (
                    Navigation::default(),
                    None,
                    nav_item.map(|nav_item| resolve_href(base, &nav_item.href)),
                )
            }
        } else if let Some(item) = nav_item {
            let path = resolve_href(base, &item.href);
            (
                parse_nav_xhtml(&read_zip_entry(archive, &path)?)?,
                Some(path),
                None,
            )
        } else {
            (Navigation::default(), None, None)
        };
    // Landmarks are a separate semantic channel from the visible TOC. Keep
    // them when an EPUB 3 package also supplies an NCX, because Guide and
    // initial-route consumers use the landmark meanings rather than the TOC
    // source selection.
    if nav_is_primary {
        let use_supplemental_items = navigation.items.is_empty();
        let mut nav = Navigation {
            items: if use_supplemental_items {
                std::mem::take(&mut navigation.items)
            } else {
                Vec::new()
            },
            page_list: std::mem::take(&mut navigation.page_list),
            landmarks: std::mem::take(&mut navigation.landmarks),
            custom: std::mem::take(&mut navigation.custom),
            ..Navigation::default()
        };
        if let Some(nav_path) = navigation_path.as_deref() {
            canonicalize_navigation(
                &mut nav,
                nav_path,
                base,
                &parsed.manifest,
                &direct_svg_spine_ids,
            )?;
        }
        navigation.landmarks = nav.landmarks;
        navigation.page_list = nav.page_list;
        navigation.custom = nav.custom;
        if use_supplemental_items {
            navigation.items = nav.items;
        }
    } else if let Some(nav_path) = supplemental_nav {
        let mut nav = parse_nav_xhtml(&read_zip_entry(archive, &nav_path)?)?;
        canonicalize_navigation(
            &mut nav,
            &nav_path,
            base,
            &parsed.manifest,
            &direct_svg_spine_ids,
        )?;
        navigation.landmarks = nav.landmarks;
        navigation.page_list = nav.page_list;
        navigation.custom = nav.custom;
        if navigation.items.is_empty() {
            navigation.items = nav.items;
        }
    }
    if let Some(path) = navigation_path {
        canonicalize_navigation(
            &mut navigation,
            &path,
            base,
            &parsed.manifest,
            &direct_svg_spine_ids,
        )?;
    }
    Ok(navigation)
}
