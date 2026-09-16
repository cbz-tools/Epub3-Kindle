mod cover;
mod css;
mod data_uri;
mod image;
mod ir;
mod normalize;

pub(crate) use cover::prepare_cover_resource;
pub(crate) use css::{project_css_for_kindle, project_inline_style_for_kindle};
pub(crate) use image::KINDLE_LD_IMAGE_MAX_BYTES;
pub use ir::{
    KindleBook, KindleLandmark, KindleLayout, KindleLayoutSemantic, KindleNavigationItem,
    KindleResource, KindleSection,
};
pub(crate) use ir::{KindlePageProgression, KindleWritingMode};
pub(crate) use normalize::normalize;
