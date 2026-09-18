use super::super::image::convert_large_image_to_jpeg;
use super::super::{KINDLE_LD_IMAGE_MAX_BYTES, KindleResource};

pub(super) fn normalize_resources(
    resources: Vec<crate::book::Resource>,
    is_comic: bool,
) -> Vec<KindleResource> {
    let mut resources: Vec<KindleResource> = resources
        .into_iter()
        .map(|resource| {
            let crate::book::Resource {
                id,
                href,
                media_type,
                properties,
                data,
            } = resource;
            KindleResource {
                id,
                href,
                media_type,
                properties,
                data,
            }
        })
        .collect();

    let eligible_indices = resources
        .iter()
        .enumerate()
        .filter_map(|(index, resource)| {
            (!is_comic
                && resource.data.len() > KINDLE_LD_IMAGE_MAX_BYTES
                && (resource.media_type.eq_ignore_ascii_case("image/jpeg")
                    || resource.media_type.eq_ignore_ascii_case("image/png")))
            .then_some(index)
        })
        .collect::<Vec<_>>();

    for (index, converted) in convert_eligible_images(&resources, &eligible_indices) {
        if let Some(converted) = converted {
            resources[index].data = converted;
            resources[index].media_type = "image/jpeg".to_owned();
        }
    }
    resources
}

fn convert_eligible_images(
    resources: &[KindleResource],
    indices: &[usize],
) -> Vec<(usize, Option<Vec<u8>>)> {
    let worker_count = image_worker_count(indices.len());
    if worker_count == 0 {
        return Vec::new();
    }

    let chunk_size = indices.len().div_ceil(worker_count);
    std::thread::scope(|scope| {
        let handles = indices
            .chunks(chunk_size)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|&index| {
                            let resource = &resources[index];
                            let converted =
                                convert_large_image_to_jpeg(&resource.data, &resource.media_type)
                                    .ok();
                            (index, converted)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();

        handles
            .into_iter()
            .filter_map(|handle| handle.join().ok())
            .flatten()
            .collect()
    })
}

fn image_worker_count(target_count: usize) -> usize {
    let available = std::thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(1);
    bounded_image_worker_count(target_count, available)
}

fn bounded_image_worker_count(target_count: usize, available: usize) -> usize {
    target_count.min((available / 2).max(1))
}

pub(super) fn project_css_resources(resources: &mut [KindleResource]) {
    for resource in resources {
        if resource.media_type.eq_ignore_ascii_case("text/css") {
            resource.data = crate::kindle::project_css_for_kindle(
                std::str::from_utf8(&resource.data)
                    .expect("EPUB CSS resources are normalized to UTF-8"),
            )
            .into_bytes();
        }
    }
}
