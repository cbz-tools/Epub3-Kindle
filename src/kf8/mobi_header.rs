#[derive(Debug, Clone)]
pub struct MobiHeader {
    pub header_length: u32,
    pub mobi_type: u32,
    pub codepage: u32,
    pub uid: u32,
    pub version: u32,
    pub first_non_text_record: u32,
    /// Compatibility alias for the standard first-image record field.
    pub first_resource_record: u32,
    pub first_image_index: u32,
    pub title_offset: u32,
    pub title_length: u32,
    pub language: u32,
    pub min_version: u32,
    pub exth_flags: u32,
    pub fcis_record: u32,
    pub fcis_count: u32,
    pub flis_record: u32,
    pub flis_count: u32,
    pub last_image_index: u16,
    /// KF7's content range occupies the same header words as KF8's FDST
    /// pointer/count pair. `None` preserves the KF8 encoding.
    pub content_record_range: Option<(u16, u16)>,
    pub extra_data_flags: u16,
    pub fdst_record: u32,
    pub fdst_flow_count: u32,
    pub index_record: u32,
    pub ncx_record: u32,
    pub skel_record: u32,
    pub guide_record: u32,
}

impl Default for MobiHeader {
    fn default() -> Self {
        Self {
            header_length: 264,
            mobi_type: 2,
            codepage: 65001,
            uid: 1,
            version: 8,
            first_non_text_record: 0xffff_ffff,
            first_resource_record: 0xffff_ffff,
            first_image_index: 0xffff_ffff,
            title_offset: 0xffff_ffff,
            title_length: 0,
            language: 0,
            min_version: 8,
            // Bit 6 advertises EXTH; bit 4 is the capability marker used by
            // the standalone KF8 writer path in Calibre and by Kindling /
            // KindleGen.  Keep the marker semantic rather than copying any
            // generator-specific metadata.
            exth_flags: 0x50,
            fcis_record: u32::MAX,
            fcis_count: 0,
            flis_record: u32::MAX,
            flis_count: 0,
            last_image_index: u16::MAX,
            content_record_range: None,
            extra_data_flags: 0,
            fdst_record: 0xffff_ffff,
            fdst_flow_count: 0,
            index_record: 0xffff_ffff,
            ncx_record: 0xffff_ffff,
            skel_record: 0xffff_ffff,
            guide_record: 0xffff_ffff,
        }
    }
}

impl MobiHeader {
    pub fn validate(&self) -> crate::error::Result<()> {
        if self.header_length as usize != MOBI_HEADER_LEN {
            return Err(crate::error::Error::Output(
                "KF8 MOBI header length must be 264 bytes".to_owned(),
            ));
        }
        if self.version < 6 || self.min_version < 6 {
            return Err(crate::error::Error::Output(
                "MOBI header must use version 6 or newer".to_owned(),
            ));
        }
        if self.first_resource_record != u32::MAX
            && self.first_image_index != u32::MAX
            && self.first_resource_record != self.first_image_index
        {
            return Err(crate::error::Error::Output(
                "MOBI first resource/image pointers disagree".to_owned(),
            ));
        }
        if let Some((first, last)) = self.content_record_range {
            if first > last {
                return Err(crate::error::Error::Output(
                    "MOBI content record range is reversed".to_owned(),
                ));
            }
        }
        Ok(())
    }

    pub fn validate_for_record_count(&self, record_count: usize) -> crate::error::Result<()> {
        self.validate()?;
        for (name, pointer) in [
            ("first_non_text_record", self.first_non_text_record),
            ("first_resource_record", self.first_resource_record),
            ("first_image_index", self.first_image_index),
            ("fcis_record", self.fcis_record),
            ("flis_record", self.flis_record),
            ("fdst_record", self.fdst_record),
            ("index_record", self.index_record),
            ("ncx_record", self.ncx_record),
            ("skel_record", self.skel_record),
            ("guide_record", self.guide_record),
        ] {
            if pointer != u32::MAX && pointer as usize >= record_count {
                return Err(crate::error::Error::Output(format!(
                    "MOBI {name} pointer {pointer} is outside {record_count} records"
                )));
            }
        }
        if self.fdst_record == u32::MAX && self.fdst_flow_count != 0 {
            return Err(crate::error::Error::Output(
                "MOBI FDST flow count requires an FDST pointer".to_owned(),
            ));
        }
        if (self.fcis_record == u32::MAX) != (self.fcis_count == 0)
            || (self.flis_record == u32::MAX) != (self.flis_count == 0)
        {
            return Err(crate::error::Error::Output(
                "MOBI FCIS/FLIS pointer and count disagree".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn encode(&self) -> Vec<u8> {
        self.validate()
            .expect("MOBI invariants must hold before encoding");
        // The KF8 extension fields used by KindleUnpack reach through the
        // guide pointer at section offset 0x104 (base offset 0xf4).
        let mut bytes = vec![0u8; 0x108];
        bytes[0..4].copy_from_slice(b"MOBI");
        put_u32(&mut bytes, 0x04, self.header_length);
        put_u32(&mut bytes, 0x08, self.mobi_type);
        put_u32(&mut bytes, 0x0c, self.codepage);
        put_u32(&mut bytes, 0x10, self.uid);
        put_u32(&mut bytes, 0x14, self.version);
        put_u32(&mut bytes, 0x40, self.first_non_text_record);
        put_u32(&mut bytes, 0x44, self.title_offset);
        put_u32(&mut bytes, 0x48, self.title_length);
        put_u32(&mut bytes, 0x4c, self.language);
        put_u32(&mut bytes, 0x58, self.min_version);
        // MOBI's standard first-image field is at record offset 0x6c,
        // i.e. 0x5c relative to the MOBI magic. Keep the older public alias
        // usable for callers that have not populated first_image_index.
        let first_image = if self.first_image_index == u32::MAX {
            self.first_resource_record
        } else {
            self.first_image_index
        };
        put_u32(&mut bytes, 0x5c, first_image);
        put_u32(&mut bytes, 0x70, self.exth_flags);
        // mobi.txt documents these 40 bytes at record offsets 0x28..0x4f.
        // The encoder starts at the MOBI magic after the 16-byte PalmDOC
        // prefix, so those offsets are 0x18..0x3f here.
        bytes[0x18..0x40].fill(0xff);
        // These fields are emitted for an unencrypted book with no images.
        // Their documented record offsets are 168 and 186, respectively.
        put_u32(&mut bytes, 0x98, u32::MAX);
        put_u16(&mut bytes, 0xaa, self.last_image_index);
        // SRCS and DATP are optional records.  A standalone KF8 that does
        // not emit either record must advertise absence with the MOBI NULL
        // index, rather than leaving the zero-filled buffer looking like a
        // record at index 0.  This is especially important for readers that
        // inspect the extended header fields before following the KF8
        // indexes; no SRCS/DATP record is created by this writer.
        put_u32(&mut bytes, 0xd0, u32::MAX);
        put_u32(&mut bytes, 0xd4, 0);
        // The remaining optional KF8 pointer slots are also NULL in the
        // standalone output.  Kindling and KindleGen use this sentinel for
        // the absent slots; retaining it here avoids a zero record alias
        // without inventing a structure for any of them.
        for offset in [0xd8, 0xdc, 0xf0, 0xf8, 0x100] {
            put_u32(&mut bytes, offset, u32::MAX);
        }
        put_u32(&mut bytes, 0xb8, self.fcis_record);
        put_u32(&mut bytes, 0xbc, self.fcis_count);
        put_u32(&mut bytes, 0xc0, self.flis_record);
        put_u32(&mut bytes, 0xc4, self.flis_count);
        put_u16(&mut bytes, 0xe2, self.extra_data_flags);
        if let Some((first_content, last_content)) = self.content_record_range {
            // KF7 stores its first/last content record numbers as u16 values
            // at whole-Record-0 offsets 0xc0/0xc2 (0xb0/0xb2 here, after
            // the 16-byte PalmDOC prefix), followed by the conventional
            // 0x00000001 at whole-Record-0 offset 0xc4.
            put_u16(&mut bytes, 0xb0, first_content);
            put_u16(&mut bytes, 0xb2, last_content);
            put_u32(&mut bytes, 0xb4, 1);
        } else {
            put_u32(&mut bytes, 0xb0, self.fdst_record);
            put_u32(&mut bytes, 0xb4, self.fdst_flow_count);
        }
        put_u32(&mut bytes, 0xe8, self.index_record);
        put_u32(&mut bytes, 0xe4, self.ncx_record);
        put_u32(&mut bytes, 0xec, self.skel_record);
        put_u32(&mut bytes, 0xf4, self.guide_record);
        bytes
    }
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

const MOBI_HEADER_LEN: usize = 0x108;
