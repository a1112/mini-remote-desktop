use mrd_decode::{available_decoder_descriptors, CodecKind, PixelFormat};

#[test]
fn production_software_descriptors_expose_planar_fast_paths_before_rgb_fallbacks() {
    let descriptors = available_decoder_descriptors();
    for (id, codec, expected_formats) in [
        (
            "software_hevc",
            CodecKind::Hevc,
            &[PixelFormat::Rgb24, PixelFormat::I420][..],
        ),
        (
            "software_hevc_main10",
            CodecKind::HevcMain10,
            &[PixelFormat::P010][..],
        ),
        (
            "software_av1",
            CodecKind::Av1,
            &[PixelFormat::Rgb24, PixelFormat::I420][..],
        ),
        (
            "software_vvc",
            CodecKind::Vvc,
            &[PixelFormat::Rgb24, PixelFormat::I420, PixelFormat::P010][..],
        ),
    ] {
        let descriptor = descriptors
            .iter()
            .find(|descriptor| descriptor.id == id)
            .unwrap_or_else(|| panic!("missing descriptor {id}"));

        assert_eq!(descriptor.codec, codec);
        assert_eq!(descriptor.output_formats, expected_formats);
    }
}

#[test]
fn ffmpeg_descriptors_are_exposed_as_fallback_decoders() {
    let descriptors = available_decoder_descriptors();
    for (id, codec, output) in [
        ("ffmpeg_h264", CodecKind::H264, PixelFormat::Nv12),
        ("ffmpeg_hevc", CodecKind::Hevc, PixelFormat::Nv12),
        ("ffmpeg_vvc", CodecKind::Vvc, PixelFormat::I420),
    ] {
        let descriptor = descriptors
            .iter()
            .find(|descriptor| descriptor.id == id)
            .unwrap_or_else(|| panic!("missing descriptor {id}"));

        assert_eq!(descriptor.codec, codec);
        assert!(descriptor.output_formats.contains(&output));
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
        Err(error) if error.to_string().contains("produced a 8-bit bitstream") => return,
        Err(error) => panic!("encode HEVC Main10 frame: {error}"),
    };
    let access_unit = access_units
        .into_iter()
        .next()
        .expect("HEVC Main10 access unit");
    let mut decoder =
        mrd_decode::create_decoder("software_hevc_main10").expect("software Main10 decoder");

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
        let error = mrd_decode::create_decoder(id)
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
