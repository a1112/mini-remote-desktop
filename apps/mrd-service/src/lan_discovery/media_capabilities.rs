use super::protocol::{
    LAN_INPUT_CONTROL_CAPABILITY, LAN_QUIC_MEDIA_V2_TRANSPORT, LAN_QUIC_MEDIA_V3_TRANSPORT,
    LAN_QUIC_PERSISTENT_MEDIA_60FPS_TRANSPORT, LAN_QUIC_PERSISTENT_MEDIA_TRANSPORT,
    LAN_QUIC_RELIABLE_MEDIA_TRANSPORT, LAN_QUIC_TRANSPORT_MUX_V1,
};

#[cfg(windows)]
pub(super) const LAN_CAPTURE_DXGI_CAPABILITY: &str = "dxgi_capture";
#[cfg(windows)]
pub(super) const LAN_ENCODE_NVENC_H264_CAPABILITY: &str = "nvenc_h264";
#[cfg(windows)]
pub(super) const LAN_ENCODE_NVENC_HEVC_CAPABILITY: &str = "encode.nvenc_hevc";
#[cfg(windows)]
pub(super) const LAN_ENCODE_NVENC_HEVC_MAIN10_CAPABILITY: &str = "encode.nvenc_hevc_main10";
#[cfg(windows)]
pub(super) const LAN_ENCODE_NVENC_AV1_CAPABILITY: &str = "encode.nvenc_av1";
#[cfg(windows)]
pub(super) const LAN_DECODE_NVDEC_CAPABILITY: &str = "nvdec";
#[cfg(windows)]
pub(super) const LAN_DECODE_NVDEC_HEVC_CAPABILITY: &str = "decode.nvdec_hevc";
#[cfg(windows)]
pub(super) const LAN_DECODE_NVDEC_HEVC_MAIN10_CAPABILITY: &str = "decode.nvdec_hevc_main10";
#[cfg(windows)]
pub(super) const LAN_DECODE_NVDEC_AV1_CAPABILITY: &str = "decode.nvdec_av1";

pub(super) const LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY: &str = "media.hevc_main_420_8bit";
pub(super) const LAN_MEDIA_HEVC_MAIN10_420_10BIT_CAPABILITY: &str = "media.hevc_main10_420_10bit";
pub(super) const LAN_MEDIA_AV1_MAIN_420_8BIT_CAPABILITY: &str = "media.av1_main_420_8bit";
pub(super) const LAN_MEDIA_COLOR_MODE_CAPABILITY: &str = "media.color_mode_v1";

#[cfg(windows)]
pub(super) const LAN_RENDER_D3D11_NATIVE_CAPABILITY: &str = "d3d11_native_render";
#[cfg(windows)]
pub(super) const LAN_RENDER_D3D11_SHARED_NV12_CAPABILITY: &str = "render.d3d11_shared_nv12";

#[cfg(target_os = "macos")]
pub(super) const LAN_CAPTURE_MACOS_CAPABILITY: &str = "macos_capture";
#[cfg(target_os = "macos")]
pub(super) const LAN_ENCODE_VIDEOTOOLBOX_H264_CAPABILITY: &str = "videotoolbox_h264";
#[cfg(target_os = "macos")]
pub(super) const LAN_ENCODE_VIDEOTOOLBOX_HEVC_CAPABILITY: &str = "videotoolbox_hevc";
#[cfg(target_os = "macos")]
pub(super) const LAN_DECODE_VIDEOTOOLBOX_CAPABILITY: &str = "videotoolbox";
#[cfg(target_os = "macos")]
pub(super) const LAN_DECODE_VIDEOTOOLBOX_H264_CAPABILITY: &str = "decode.videotoolbox_h264";
#[cfg(target_os = "macos")]
pub(super) const LAN_DECODE_VIDEOTOOLBOX_HEVC_CAPABILITY: &str = "decode.videotoolbox_hevc";
#[cfg(target_os = "macos")]
pub(super) const LAN_RENDER_MACOS_NATIVE_CAPABILITY: &str = "macos_native_render";

pub(super) fn lan_media_capabilities() -> Vec<String> {
    // Task 19 re-enables this only with authenticated ControlEnvelopeV2.
    lan_media_capabilities_with_input_control(false)
}

pub(super) fn lan_media_capabilities_with_input_control(
    input_control_available: bool,
) -> Vec<String> {
    let mut capabilities = vec![
        LAN_QUIC_MEDIA_V2_TRANSPORT.to_string(),
        LAN_QUIC_MEDIA_V3_TRANSPORT.to_string(),
        LAN_QUIC_RELIABLE_MEDIA_TRANSPORT.to_string(),
        LAN_QUIC_PERSISTENT_MEDIA_TRANSPORT.to_string(),
        LAN_QUIC_TRANSPORT_MUX_V1.to_string(),
        LAN_QUIC_PERSISTENT_MEDIA_60FPS_TRANSPORT.to_string(),
    ];
    #[cfg(windows)]
    {
        capabilities.extend([
            LAN_CAPTURE_DXGI_CAPABILITY.to_string(),
            LAN_ENCODE_NVENC_H264_CAPABILITY.to_string(),
            LAN_ENCODE_NVENC_HEVC_CAPABILITY.to_string(),
            LAN_ENCODE_NVENC_HEVC_MAIN10_CAPABILITY.to_string(),
            LAN_ENCODE_NVENC_AV1_CAPABILITY.to_string(),
            LAN_DECODE_NVDEC_CAPABILITY.to_string(),
            LAN_DECODE_NVDEC_HEVC_CAPABILITY.to_string(),
            LAN_DECODE_NVDEC_HEVC_MAIN10_CAPABILITY.to_string(),
            LAN_DECODE_NVDEC_AV1_CAPABILITY.to_string(),
            LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY.to_string(),
            LAN_MEDIA_HEVC_MAIN10_420_10BIT_CAPABILITY.to_string(),
            LAN_MEDIA_AV1_MAIN_420_8BIT_CAPABILITY.to_string(),
            LAN_MEDIA_COLOR_MODE_CAPABILITY.to_string(),
            LAN_RENDER_D3D11_NATIVE_CAPABILITY.to_string(),
            LAN_RENDER_D3D11_SHARED_NV12_CAPABILITY.to_string(),
            crate::display_mode::capability_name().to_string(),
        ]);
    }
    #[cfg(target_os = "macos")]
    {
        capabilities.extend(macos_lan_media_capabilities());
    }
    #[cfg(target_os = "linux")]
    {
        capabilities.extend([
            "pipewire_capture".to_string(),
            "openh264_fallback".to_string(),
            "software_decode".to_string(),
        ]);
    }
    #[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
    {
        capabilities.extend([
            "openh264_fallback".to_string(),
            "software_decode".to_string(),
        ]);
    }
    if input_control_available {
        capabilities.push(LAN_INPUT_CONTROL_CAPABILITY.to_string());
    }
    capabilities
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy)]
pub(super) struct MacosLanMediaCapabilityProbe {
    pub(super) videotoolbox_h264_encoder: bool,
    pub(super) videotoolbox_hevc_encoder: bool,
    pub(super) videotoolbox_h264_decoder: bool,
    pub(super) videotoolbox_hevc_decoder: bool,
}

#[cfg(target_os = "macos")]
fn macos_lan_media_capabilities() -> Vec<String> {
    use std::sync::OnceLock;

    static MACOS_LAN_MEDIA_CAPABILITIES: OnceLock<Vec<String>> = OnceLock::new();
    MACOS_LAN_MEDIA_CAPABILITIES
        .get_or_init(|| {
            macos_lan_media_capabilities_from_probe(probe_macos_lan_media_capabilities())
        })
        .clone()
}

#[cfg(target_os = "macos")]
pub(super) fn probe_macos_lan_media_capabilities() -> MacosLanMediaCapabilityProbe {
    MacosLanMediaCapabilityProbe {
        videotoolbox_h264_encoder: mrd_codec_videotoolbox::VideoToolboxH264Encoder::new(
            640, 480, 30,
        )
        .is_ok(),
        videotoolbox_hevc_encoder: mrd_codec_videotoolbox::VideoToolboxHevcEncoder::new(
            640, 480, 30,
        )
        .is_ok(),
        videotoolbox_h264_decoder: videotoolbox_decoder_enabled()
            && mrd_codec_videotoolbox::VideoToolboxH264Decoder::new().is_ok(),
        videotoolbox_hevc_decoder: videotoolbox_decoder_enabled()
            && mrd_codec_videotoolbox::VideoToolboxHevcDecoder::new().is_ok(),
    }
}

#[cfg(target_os = "macos")]
pub(super) fn macos_lan_media_capabilities_from_probe(
    probe: MacosLanMediaCapabilityProbe,
) -> Vec<String> {
    let mut capabilities = vec![
        LAN_CAPTURE_MACOS_CAPABILITY.to_string(),
        LAN_RENDER_MACOS_NATIVE_CAPABILITY.to_string(),
        "openh264_fallback".to_string(),
        "software_decode".to_string(),
    ];
    if probe.videotoolbox_h264_encoder {
        capabilities.push(LAN_ENCODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string());
    }
    if probe.videotoolbox_hevc_encoder {
        capabilities.push(LAN_ENCODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string());
        capabilities.push(LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY.to_string());
    }
    if probe.videotoolbox_h264_decoder {
        capabilities.push(LAN_DECODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string());
    }
    if probe.videotoolbox_hevc_decoder {
        capabilities.push(LAN_DECODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string());
    }
    if probe.videotoolbox_h264_decoder && probe.videotoolbox_hevc_decoder {
        capabilities.push(LAN_DECODE_VIDEOTOOLBOX_CAPABILITY.to_string());
    }
    capabilities
}

#[cfg(target_os = "macos")]
fn videotoolbox_decoder_enabled() -> bool {
    !matches!(
        std::env::var("MRD_DISABLE_VIDEOTOOLBOX_DECODER").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_capabilities_follow_input_control_availability() {
        assert!(lan_media_capabilities_with_input_control(true)
            .contains(&LAN_INPUT_CONTROL_CAPABILITY.to_string()));
        assert!(!lan_media_capabilities_with_input_control(false)
            .contains(&LAN_INPUT_CONTROL_CAPABILITY.to_string()));
    }
}
