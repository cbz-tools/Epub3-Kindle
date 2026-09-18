use std::process::Command;

use crate::audit_support::epub;
use crate::audit_support::semantic::{TargetProjection, decode_embed_number};
use crate::audit_support::temp::TempDir;

#[test]
fn epc003_legacy_woff_media_type_preserves_idpf_obfuscated_font_resource() {
    let temp = TempDir::new("epc003-legacy-woff");
    for (variant, media_type) in [
        ("legacy", "application/font-woff"),
        ("standard", "font/woff"),
    ] {
        let (input, expected_woff) = epub::epc003_woff_idpf_obfuscated(media_type);
        let input_path = temp.write(&format!("epc003-{variant}-woff.epub"), &input);
        let output_path = temp.path().join(format!("epc003-{variant}-woff.azw3"));
        let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
            .arg(&input_path)
            .arg("-o")
            .arg(&output_path)
            .output()
            .expect("EPC-003 CLI process starts");
        assert_eq!(
            cli.status.code(),
            Some(0),
            "{media_type} must convert successfully: {}",
            String::from_utf8_lossy(&cli.stderr)
        );
        assert!(
            cli.stderr.is_empty(),
            "{media_type} conversion must not warn: {}",
            String::from_utf8_lossy(&cli.stderr)
        );

        let artifact = std::fs::read(&output_path).expect("CLI writes the AZW3 artifact");
        let target = TargetProjection::parse(&artifact).expect("inspect generated Kindle content");
        assert!(target.body_text().contains("AUTH_EPC003_WOFF_FONT"));
        assert_eq!(
            target.font_records.len(),
            1,
            "one font FONT record is emitted"
        );
        let target_woff = target
            .decompressed_font(target.font_records[0])
            .expect("decode the generated Kindle FONT container");
        assert_eq!(
            target_woff, expected_woff,
            "IDPF obfuscation is reversed and WOFF payload is transported without transcoding"
        );
        assert!(target_woff.starts_with(b"wOFF"));

        let font_reference = target
            .css
            .split("src:url(")
            .nth(1)
            .and_then(|value| value.split(')').next())
            .expect("generated @font-face source is present");
        assert!(font_reference.starts_with("kindle:embed:"));
        assert!(font_reference.contains(&format!("mime={media_type}")));
        let embed = decode_embed_number(
            font_reference
                .strip_prefix("kindle:embed:")
                .unwrap()
                .split('?')
                .next()
                .unwrap(),
        )
        .expect("font uses a Kindle resource number");
        assert_eq!(embed, 1);
        assert_eq!(
            target.resource_bytes(embed).unwrap(),
            target.db.record(target.font_records[0]).unwrap(),
            "CSS reference points to the emitted font resource"
        );
    }
}
