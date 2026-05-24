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
fn production_software_codecs_do_not_depend_on_ffmpeg() {
    for (id, expected_runtime) in [
        ("software_hevc", "rust_h265"),
        ("software_hevc_main10", "rust_h265"),
        ("software_av1", "dav1d"),
        ("software_h266", "vvdec"),
    ] {
        let error = create_decoder(id)
            .err()
            .unwrap_or_else(|| panic!("{id} unexpectedly available in the default test build"));
        let message = error.to_string().to_ascii_lowercase();
        assert!(
            !message.contains("ffmpeg"),
            "{id} unavailable message must not mention ffmpeg: {message}"
        );
        assert!(
            message.contains(expected_runtime),
            "{id} unavailable message should name {expected_runtime}: {message}"
        );
    }
}
