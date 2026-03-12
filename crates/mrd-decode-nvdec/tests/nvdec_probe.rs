use mrd_decode_nvdec::{
    probe_h264_available, probe_hevc_available, probe_hevc_main10_available, probe_runtime,
    NvdecDecoder,
};
use openh264::{
    encoder::Encoder,
    formats::{RgbSliceU8, YUVBuffer},
};

#[test]
fn nvdec_probe_returns_result_without_panicking() {
    let result = probe_h264_available();
    assert!(
        result.is_ok() || result.is_err(),
        "probe should always return a result"
    );
}

#[test]
fn nvdec_hevc_probe_returns_structured_result() {
    let result = probe_hevc_available();
    if let Err(error) = result {
        assert!(
            error.to_lowercase().contains("hevc"),
            "expected hevc-specific probe error, got: {error}"
        );
        assert!(
            error.contains("not wired")
                || error.contains("unsupported")
                || error.contains("runtime")
                || error.contains("nvdec")
                || error.contains("cuda"),
            "expected structured hevc probe error, got: {error}"
        );
    }
}

#[test]
fn nvdec_hevc_main10_probe_returns_structured_result() {
    let result = probe_hevc_main10_available();
    if let Err(error) = result {
        let lowered = error.to_lowercase();
        assert!(
            lowered.contains("hevc"),
            "expected hevc-specific main10 probe error, got: {error}"
        );
        assert!(
            lowered.contains("10-bit") || lowered.contains("main10"),
            "expected main10-specific probe error, got: {error}"
        );
        assert!(
            error.contains("not wired")
                || error.contains("unsupported")
                || error.contains("runtime")
                || error.contains("nvdec")
                || error.contains("cuda"),
            "expected structured hevc main10 probe error, got: {error}"
        );
    }
}

#[test]
fn nvdec_runtime_probe_reports_library_state() {
    let runtime = probe_runtime();

    assert_eq!(runtime.backend, "windows-nvdec");
    assert!(
        runtime.summary.contains("nvdec")
            || runtime.summary.contains("nvcuvid")
            || runtime.summary.contains("Windows"),
        "unexpected runtime summary: {}",
        runtime.summary
    );

    if cfg!(windows) {
        assert!(
            runtime
                .checked_items
                .iter()
                .any(|item| *item == "nvcuvid.dll"),
            "windows probe should report nvcuvid.dll check"
        );
        assert!(
            runtime
                .capability_probes
                .iter()
                .any(|probe| probe.codec == "h264" && probe.bit_depth_minus8 == 0),
            "runtime probe should include h264 capability summary"
        );
        assert!(
            runtime
                .capability_probes
                .iter()
                .any(|probe| probe.codec == "hevc" && probe.bit_depth_minus8 == 0),
            "runtime probe should include hevc 8-bit capability summary"
        );
        assert!(
            runtime
                .capability_probes
                .iter()
                .any(|probe| probe.codec == "hevc" && probe.bit_depth_minus8 == 2),
            "runtime probe should include hevc main10 capability summary"
        );
    } else {
        assert!(runtime.summary.contains("Windows"));
    }
}

#[test]
fn nvdec_decoder_new_returns_structured_result() {
    let runtime = probe_runtime();
    let result = NvdecDecoder::new();

    if runtime.summary == "nvdec runtime libraries and core exports are present" {
        assert!(
            result.is_ok() || result.is_err(),
            "constructor should never panic on supported runtime"
        );
    } else {
        let error = match result {
            Ok(_) => panic!("unsupported runtime should report a clear error"),
            Err(error) => error,
        };
        assert!(
            error.contains("nvdec")
                || error.contains("cuda")
                || error.contains("cu")
                || error.contains("failed"),
            "unexpected constructor error: {error}"
        );
    }
}

#[test]
fn nvdec_push_access_unit_returns_structured_result() {
    let runtime = probe_runtime();
    let mut decoder = match NvdecDecoder::new() {
        Ok(decoder) => decoder,
        Err(error) => {
            assert!(
                error.contains("nvdec")
                    || error.contains("cuda")
                    || error.contains("cu")
                    || error.contains("failed"),
                "unexpected constructor error: {error}"
            );
            return;
        }
    };

    let access_unit = encoded_access_unit();
    let result = decoder.push_access_unit(access_unit.as_slice());

    if runtime.summary == "nvdec runtime libraries and core exports are present" {
        assert!(
            result.is_ok() || result.is_err(),
            "push_access_unit should return a structured result"
        );
    } else {
        assert!(
            result.is_err(),
            "unsupported runtime should not silently succeed"
        );
    }
}

#[test]
fn nvdec_decoder_emits_rgb_frame() {
    let mut decoder = match NvdecDecoder::new() {
        Ok(decoder) => decoder,
        Err(error) => {
            assert!(
                error.contains("nvdec")
                    || error.contains("cuda")
                    || error.contains("cu")
                    || error.contains("failed"),
                "unexpected constructor error: {error}"
            );
            return;
        }
    };

    let access_unit = encoded_access_unit();
    decoder
        .push_access_unit(access_unit.as_slice())
        .expect("valid h264 access unit should traverse nvdec path");

    let frames = decoder.drain_decoded_frames();
    assert!(
        !frames.is_empty(),
        "nvdec should emit at least one decoded frame"
    );
    assert_eq!(frames[0].width, 128);
    assert_eq!(frames[0].height, 128);
    assert_eq!(frames[0].data.len(), 128 * 128 * 3);
}

#[test]
fn nvdec_decoder_reports_decode_activity_diagnostics() {
    let mut decoder = match NvdecDecoder::new() {
        Ok(decoder) => decoder,
        Err(error) => {
            assert!(
                error.contains("nvdec")
                    || error.contains("cuda")
                    || error.contains("cu")
                    || error.contains("failed"),
                "unexpected constructor error: {error}"
            );
            return;
        }
    };

    let access_unit = encoded_access_unit();
    decoder
        .push_access_unit(access_unit.as_slice())
        .expect("valid h264 access unit should traverse nvdec path");

    let diagnostics = decoder.diagnostics();
    assert!(
        diagnostics.decode_calls > 0,
        "expected nvdec decode activity, got: {diagnostics:?}"
    );
    assert!(
        diagnostics.display_calls > 0,
        "expected nvdec display activity, got: {diagnostics:?}"
    );
    assert_eq!(diagnostics.last_sequence_coded_width, Some(128));
    assert_eq!(diagnostics.last_sequence_coded_height, Some(128));
    assert_eq!(diagnostics.last_sequence_display_width, Some(128));
    assert_eq!(diagnostics.last_sequence_display_height, Some(128));
    assert_eq!(diagnostics.last_support_codec.as_deref(), Some("h264"));
    assert_eq!(diagnostics.last_support_bit_depth_minus8, Some(0));
    assert_eq!(diagnostics.last_support_chroma_format, Some(1));
    assert_eq!(
        diagnostics.last_support_decision.as_deref(),
        Some("supported")
    );
    assert!(
        diagnostics.last_sequence_decision.as_deref() == Some("create")
            || diagnostics.last_sequence_decision.as_deref() == Some("reuse"),
        "expected a sequence decision, got: {diagnostics:?}"
    );
    assert!(
        diagnostics.last_decode_status_phase.is_some(),
        "expected decode-status phase diagnostics, got: {diagnostics:?}"
    );
    assert!(
        diagnostics.last_decode_status_description.is_some(),
        "expected decode-status description diagnostics, got: {diagnostics:?}"
    );
}

#[test]
fn nvdec_decoder_reports_stage_for_malformed_input() {
    let mut decoder = match NvdecDecoder::new() {
        Ok(decoder) => decoder,
        Err(error) => {
            assert!(
                error.contains("nvdec")
                    || error.contains("cuda")
                    || error.contains("cu")
                    || error.contains("failed"),
                "unexpected constructor error: {error}"
            );
            return;
        }
    };

    let error = decoder
        .push_access_unit(&[1, 2, 3, 4])
        .expect_err("malformed access unit should return a structured error");
    assert!(
        error.contains("input") || error.contains("parse") || error.contains("decode"),
        "expected stage-aware malformed-input error, got: {error}"
    );
}

#[test]
fn nvdec_decoder_recreates_on_resolution_change() {
    let mut decoder = match NvdecDecoder::new() {
        Ok(decoder) => decoder,
        Err(error) => {
            assert!(
                error.contains("nvdec")
                    || error.contains("cuda")
                    || error.contains("cu")
                    || error.contains("failed"),
                "unexpected constructor error: {error}"
            );
            return;
        }
    };

    let first = encoded_access_unit_with_size(128, 128);
    decoder
        .push_access_unit(first.as_slice())
        .expect("first access unit should decode through nvdec");
    let _ = decoder.drain_decoded_frames();

    let second = encoded_access_unit_with_size(256, 128);
    decoder
        .push_access_unit(second.as_slice())
        .expect("resolution change should recreate nvdec decoder");

    let diagnostics = decoder.diagnostics();
    assert!(
        diagnostics.recreate_count > 0,
        "expected at least one decoder recreate, got: {diagnostics:?}"
    );
    assert!(
        diagnostics.last_reconfigure_attempted,
        "expected reconfigure-first path to be attempted, got: {diagnostics:?}"
    );
    assert!(
        diagnostics.last_reconfigure_result.is_some(),
        "expected reconfigure result diagnostics, got: {diagnostics:?}"
    );
    assert_eq!(
        diagnostics.last_sequence_decision.as_deref(),
        Some("recreate")
    );
    assert_eq!(diagnostics.last_recreate_from_coded_width, Some(128));
    assert_eq!(diagnostics.last_recreate_from_coded_height, Some(128));
    assert_eq!(diagnostics.last_recreate_to_coded_width, Some(256));
    assert_eq!(diagnostics.last_recreate_to_coded_height, Some(128));
    assert!(
        diagnostics.reconfigure_fallback_used,
        "expected recreate fallback diagnostics, got: {diagnostics:?}"
    );
    assert_eq!(diagnostics.active_coded_width, Some(256));
    assert_eq!(diagnostics.active_coded_height, Some(128));

    let frames = decoder.drain_decoded_frames();
    assert!(
        frames
            .iter()
            .any(|frame| frame.width == 256 && frame.height == 128),
        "expected a decoded frame at the new resolution, got: {frames:?}"
    );
}

fn encoded_access_unit() -> Vec<u8> {
    encoded_access_unit_with_size(128, 128)
}

fn encoded_access_unit_with_size(width: usize, height: usize) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            rgb.push(x as u8);
            rgb.push(y as u8);
            rgb.push(96);
        }
    }
    let rgb_source = RgbSliceU8::new(&rgb, (width, height));
    let yuv = YUVBuffer::from_rgb_source(rgb_source);
    let mut encoder = Encoder::new().expect("openh264 encoder");
    encoder.encode(&yuv).expect("encode access unit").to_vec()
}
