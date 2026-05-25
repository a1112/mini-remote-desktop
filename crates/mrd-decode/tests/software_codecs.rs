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

#[cfg(all(windows, feature = "software-rust-h265"))]
#[test]
fn software_hevc_main10_decodes_nvenc_main10_access_unit() {
    use mrd_encode_nvenc::NvencHevcEncoder;
    use mrd_pipeline_core::{CapturedFrame, FramePixelFormat, VideoEncoder};

    let Ok(mut encoder) = NvencHevcEncoder::new_main10_with_bitrate(320, 240, 30, 8_000_000) else {
        return;
    };
    let frame = CapturedFrame::from_cpu(
        320,
        240,
        FramePixelFormat::Bgra32,
        33_000,
        vec![0x80; 320 * 240 * 4],
    );
    let access_units = match encoder.encode(&frame) {
        Ok(access_units) => access_units,
        Err(error) if error.to_string().contains("UnsupportedParam") => return,
        Err(error) => panic!("encode HEVC Main10 frame: {error}"),
    };
    let access_unit = access_units
        .into_iter()
        .next()
        .expect("HEVC Main10 access unit");
    let mut decoder = create_decoder("software_hevc_main10").expect("software Main10 decoder");

    decoder
        .push_access_unit(&access_unit.bytes)
        .expect("decode HEVC Main10 access unit");
    let frames = decoder.drain_decoded_frames();

    assert!(!frames.is_empty(), "Main10 decoder produced no frames");
}

#[cfg(not(any(
    feature = "software-rust-h265",
    feature = "software-dav1d",
    feature = "software-vvdec"
)))]
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
