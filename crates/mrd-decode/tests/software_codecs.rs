use mrd_decode::{available_decoder_descriptors, create_decoder, CodecKind, PixelFormat};

#[test]
fn software_hevc_av1_and_vvc_descriptors_are_exposed_as_rgb24_decoders() {
    let descriptors = available_decoder_descriptors();
    for (id, codec) in [
        ("software_hevc", CodecKind::Hevc),
        ("software_hevc_main10", CodecKind::HevcMain10),
        ("software_av1", CodecKind::Av1),
        ("software_vvc", CodecKind::Vvc),
    ] {
        let descriptor = descriptors
            .iter()
            .find(|descriptor| descriptor.id == id)
            .unwrap_or_else(|| panic!("missing descriptor {id}"));

        assert_eq!(descriptor.codec, codec);
        assert!(descriptor.output_formats.contains(&PixelFormat::Rgb24));
    }
}

#[test]
fn software_hevc_creation_is_runtime_probed_not_compile_gated() {
    match create_decoder("software_hevc") {
        Ok(mut decoder) => {
            let result = decoder.push_access_unit(&[0, 1, 2, 3]);
            assert!(result.is_err());
        }
        Err(error) => {
            let message = error.to_string().to_ascii_lowercase();
            assert!(
                message.contains("ffmpeg"),
                "software HEVC unavailable should name ffmpeg: {message}"
            );
        }
    }
}

#[test]
fn software_vvc_aliases_report_ffmpeg_or_vvdec_runtime_boundary() {
    let error = create_decoder("software_h266")
        .err()
        .map(|error| error.to_string());

    if let Some(message) = error {
        let lower = message.to_ascii_lowercase();
        assert!(
            lower.contains("ffmpeg") || lower.contains("vvdec"),
            "software VVC unavailable should name the external runtime: {message}"
        );
    }
}
