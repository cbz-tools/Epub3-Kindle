use crate::kf8::MobiHeader;

pub(crate) struct LegacyHeaderFields {
    pub(crate) first_non_text_record: u32,
    pub(crate) first_resource_record: u32,
    pub(crate) fcis_record: u32,
    pub(crate) flis_record: u32,
    pub(crate) last_image_index: u16,
    pub(crate) content_record_range: Option<(u16, u16)>,
    pub(crate) extra_data_flags: u16,
    pub(crate) language: u32,
    pub(crate) uid: u32,
    pub(crate) ncx_record: u32,
    pub(crate) exth_flags: u32,
}

pub(crate) fn new(fields: LegacyHeaderFields) -> MobiHeader {
    MobiHeader {
        version: 6,
        min_version: 6,
        first_non_text_record: fields.first_non_text_record,
        first_resource_record: fields.first_resource_record,
        first_image_index: fields.first_resource_record,
        fcis_record: fields.fcis_record,
        fcis_count: 1,
        flis_record: fields.flis_record,
        flis_count: 1,
        last_image_index: fields.last_image_index,
        content_record_range: fields.content_record_range,
        extra_data_flags: fields.extra_data_flags,
        language: fields.language,
        uid: fields.uid,
        fdst_record: u32::MAX,
        fdst_flow_count: 0,
        index_record: u32::MAX,
        ncx_record: fields.ncx_record,
        skel_record: u32::MAX,
        guide_record: u32::MAX,
        exth_flags: fields.exth_flags,
        ..MobiHeader::default()
    }
}
