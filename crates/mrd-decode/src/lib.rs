pub use mrd_pipeline_core::{
    DecodedFrame as CoreDecodedFrame, DecodedFrameData, PipelineError, RuntimeStatus, VideoDecoder,
};
use openh264::{
    decoder::{DecodedYUV, Decoder as OpenH264Decoder},
    formats::YUVSource,
};
#[cfg(target_os = "linux")]
use std::{
    io::{Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    thread,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecKind {
    H264,
    Hevc,
    HevcMain10,
    Av1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb24,
    Bgra32,
    D3d11Texture,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecoderDescriptor {
    pub id: &'static str,
    pub codec: CodecKind,
    pub runtime_status: RuntimeStatus,
    pub output_formats: &'static [PixelFormat],
}

const RGB24_OUTPUTS: &[PixelFormat] = &[PixelFormat::Rgb24];
const D3D11_TEXTURE_OUTPUTS: &[PixelFormat] = &[PixelFormat::D3d11Texture];

const H264_SOFTWARE_DESCRIPTOR: DecoderDescriptor = DecoderDescriptor {
    id: "h264_software",
    codec: CodecKind::H264,
    runtime_status: RuntimeStatus::RuntimeBacked,
    output_formats: RGB24_OUTPUTS,
};

const NVDEC_DESCRIPTOR: DecoderDescriptor = DecoderDescriptor {
    id: "nvdec",
    codec: CodecKind::H264,
    runtime_status: RuntimeStatus::RuntimeBacked,
    output_formats: RGB24_OUTPUTS,
};

const NVDEC_D3D11_SHARED_DESCRIPTOR: DecoderDescriptor = DecoderDescriptor {
    id: "nvdec_d3d11_shared",
    codec: CodecKind::H264,
    runtime_status: RuntimeStatus::RuntimeBacked,
    output_formats: D3D11_TEXTURE_OUTPUTS,
};

const NVDEC_AV1_DESCRIPTOR: DecoderDescriptor = DecoderDescriptor {
    id: "nvdec_av1",
    codec: CodecKind::Av1,
    runtime_status: RuntimeStatus::RuntimeBacked,
    output_formats: RGB24_OUTPUTS,
};

const NVDEC_HEVC_DESCRIPTOR: DecoderDescriptor = DecoderDescriptor {
    id: "nvdec_hevc",
    codec: CodecKind::Hevc,
    runtime_status: RuntimeStatus::RuntimeBacked,
    output_formats: RGB24_OUTPUTS,
};

const NVDEC_HEVC_D3D11_SHARED_DESCRIPTOR: DecoderDescriptor = DecoderDescriptor {
    id: "nvdec_hevc_d3d11_shared",
    codec: CodecKind::Hevc,
    runtime_status: RuntimeStatus::RuntimeBacked,
    output_formats: D3D11_TEXTURE_OUTPUTS,
};

const NVDEC_HEVC_MAIN10_DESCRIPTOR: DecoderDescriptor = DecoderDescriptor {
    id: "nvdec_hevc_main10",
    codec: CodecKind::HevcMain10,
    runtime_status: RuntimeStatus::RuntimeBacked,
    output_formats: RGB24_OUTPUTS,
};

const NVDEC_HEVC_MAIN10_D3D11_SHARED_DESCRIPTOR: DecoderDescriptor = DecoderDescriptor {
    id: "nvdec_hevc_main10_d3d11_shared",
    codec: CodecKind::HevcMain10,
    runtime_status: RuntimeStatus::RuntimeBacked,
    output_formats: D3D11_TEXTURE_OUTPUTS,
};

#[cfg(target_os = "linux")]
const LINUX_H264_DESCRIPTOR: DecoderDescriptor = DecoderDescriptor {
    id: "linux_h264",
    codec: CodecKind::H264,
    runtime_status: RuntimeStatus::RuntimeBacked,
    output_formats: RGB24_OUTPUTS,
};

#[cfg(target_os = "linux")]
const LINUX_HEVC_DESCRIPTOR: DecoderDescriptor = DecoderDescriptor {
    id: "linux_hevc",
    codec: CodecKind::Hevc,
    runtime_status: RuntimeStatus::RuntimeBacked,
    output_formats: RGB24_OUTPUTS,
};

#[cfg(target_os = "linux")]
const LINUX_HEVC_MAIN10_DESCRIPTOR: DecoderDescriptor = DecoderDescriptor {
    id: "linux_hevc_main10",
    codec: CodecKind::HevcMain10,
    runtime_status: RuntimeStatus::RuntimeBacked,
    output_formats: RGB24_OUTPUTS,
};

pub fn available_decoder_descriptors() -> Vec<DecoderDescriptor> {
    let descriptors = vec![
        H264_SOFTWARE_DESCRIPTOR.clone(),
        NVDEC_D3D11_SHARED_DESCRIPTOR.clone(),
        NVDEC_DESCRIPTOR.clone(),
        NVDEC_HEVC_D3D11_SHARED_DESCRIPTOR.clone(),
        NVDEC_HEVC_DESCRIPTOR.clone(),
        NVDEC_HEVC_MAIN10_D3D11_SHARED_DESCRIPTOR.clone(),
        NVDEC_HEVC_MAIN10_DESCRIPTOR.clone(),
        NVDEC_AV1_DESCRIPTOR.clone(),
    ];

    #[cfg(target_os = "linux")]
    descriptors.extend([
        LINUX_H264_DESCRIPTOR.clone(),
        LINUX_HEVC_DESCRIPTOR.clone(),
        LINUX_HEVC_MAIN10_DESCRIPTOR.clone(),
    ]);

    descriptors
}

pub fn create_decoder(id: &str) -> Result<Box<dyn VideoDecoder>, PipelineError> {
    match id {
        "h264_software" => Ok(Box::new(H264SoftwareDecoder::new()?)),
        "linux_h264" | "gstreamer_h264" | "vaapi_h264" => create_linux_h264_decoder(),
        "linux_hevc" | "gstreamer_hevc" | "vaapi_hevc" => create_linux_hevc_decoder(),
        "linux_hevc_main10" | "gstreamer_hevc_main10" | "vaapi_hevc_main10" => {
            create_linux_hevc_main10_decoder()
        }
        "nvdec" => Ok(Box::new(NvdecVideoDecoder::new()?)),
        "nvdec_d3d11_shared" => Ok(Box::new(NvdecVideoDecoder::new_d3d11_shared()?)),
        "nvdec_hevc_d3d11_shared" | "nvdec_d3d11_shared_hevc" => {
            Ok(Box::new(NvdecVideoDecoder::new_hevc_d3d11_shared()?))
        }
        "nvdec_hevc" => Ok(Box::new(NvdecVideoDecoder::new_hevc()?)),
        "nvdec_hevc_main10_d3d11_shared" | "nvdec_d3d11_shared_hevc_main10" => {
            Ok(Box::new(NvdecVideoDecoder::new_hevc_main10_d3d11_shared()?))
        }
        "nvdec_hevc_main10" => Ok(Box::new(NvdecVideoDecoder::new_hevc_main10()?)),
        "nvdec_av1" => Ok(Box::new(NvdecVideoDecoder::new_av1()?)),
        other => Err(PipelineError::Message(format!(
            "unknown decoder backend: {other}"
        ))),
    }
}

#[cfg(target_os = "linux")]
pub fn probe_linux_h264_hardware_available() -> Result<String, PipelineError> {
    let backend = select_linux_gst_backend(LinuxGstCodec::H264)?;
    Ok(backend.label.to_string())
}

#[cfg(not(target_os = "linux"))]
pub fn probe_linux_h264_hardware_available() -> Result<String, PipelineError> {
    Err(PipelineError::Message(
        "Linux H.264 hardware decode is only available on Linux".to_string(),
    ))
}

#[cfg(target_os = "linux")]
pub fn probe_linux_hevc_hardware_available() -> Result<String, PipelineError> {
    let backend = select_linux_gst_backend(LinuxGstCodec::Hevc)?;
    Ok(backend.label.to_string())
}

#[cfg(not(target_os = "linux"))]
pub fn probe_linux_hevc_hardware_available() -> Result<String, PipelineError> {
    Err(PipelineError::Message(
        "Linux HEVC hardware decode is only available on Linux".to_string(),
    ))
}

#[cfg(target_os = "linux")]
pub fn probe_linux_hevc_main10_hardware_available() -> Result<String, PipelineError> {
    let backend = select_linux_gst_backend(LinuxGstCodec::HevcMain10)?;
    Ok(backend.label.to_string())
}

#[cfg(not(target_os = "linux"))]
pub fn probe_linux_hevc_main10_hardware_available() -> Result<String, PipelineError> {
    Err(PipelineError::Message(
        "Linux HEVC Main10 hardware decode is only available on Linux".to_string(),
    ))
}

#[cfg(target_os = "linux")]
fn create_linux_h264_decoder() -> Result<Box<dyn VideoDecoder>, PipelineError> {
    Ok(Box::new(LinuxGstDecoder::new(LinuxGstCodec::H264)?))
}

#[cfg(not(target_os = "linux"))]
fn create_linux_h264_decoder() -> Result<Box<dyn VideoDecoder>, PipelineError> {
    Err(PipelineError::Message(
        "Linux H.264 hardware decode is only available on Linux".to_string(),
    ))
}

#[cfg(target_os = "linux")]
fn create_linux_hevc_decoder() -> Result<Box<dyn VideoDecoder>, PipelineError> {
    Ok(Box::new(LinuxGstDecoder::new(LinuxGstCodec::Hevc)?))
}

#[cfg(not(target_os = "linux"))]
fn create_linux_hevc_decoder() -> Result<Box<dyn VideoDecoder>, PipelineError> {
    Err(PipelineError::Message(
        "Linux HEVC hardware decode is only available on Linux".to_string(),
    ))
}

#[cfg(target_os = "linux")]
fn create_linux_hevc_main10_decoder() -> Result<Box<dyn VideoDecoder>, PipelineError> {
    Ok(Box::new(LinuxGstDecoder::new(LinuxGstCodec::HevcMain10)?))
}

#[cfg(not(target_os = "linux"))]
fn create_linux_hevc_main10_decoder() -> Result<Box<dyn VideoDecoder>, PipelineError> {
    Err(PipelineError::Message(
        "Linux HEVC Main10 hardware decode is only available on Linux".to_string(),
    ))
}

pub struct H264SoftwareDecoder {
    decoder: OpenH264Decoder,
    decoded_frames: Vec<CoreDecodedFrame>,
}

pub struct NvdecVideoDecoder {
    decoder: mrd_decode_nvdec::NvdecDecoder,
    require_shared_output: bool,
}

impl NvdecVideoDecoder {
    pub fn new() -> Result<Self, PipelineError> {
        let decoder = mrd_decode_nvdec::NvdecDecoder::new_with_output_mode(
            mrd_decode_nvdec::NvdecOutputMode::CpuNv12,
        )
        .map_err(|e| PipelineError::Message(format!("nvdec create failed: {e}")))?;
        Ok(Self {
            decoder,
            require_shared_output: false,
        })
    }

    pub fn new_d3d11_shared() -> Result<Self, PipelineError> {
        let mut decoder = mrd_decode_nvdec::NvdecDecoder::new_with_output_mode(
            mrd_decode_nvdec::NvdecOutputMode::CpuNv12,
        )
        .map_err(|e| PipelineError::Message(format!("nvdec d3d11 shared create failed: {e}")))?;
        #[cfg(windows)]
        decoder.enable_shared_texture(true);
        #[cfg(not(windows))]
        {
            return Err(PipelineError::Message(
                "nvdec d3d11 shared output is only available on Windows".to_string(),
            ));
        }
        Ok(Self {
            decoder,
            require_shared_output: true,
        })
    }

    pub fn new_av1() -> Result<Self, PipelineError> {
        let decoder = mrd_decode_nvdec::NvdecDecoder::new_av1_with_output_mode(
            mrd_decode_nvdec::NvdecOutputMode::CpuNv12,
        )
        .map_err(|e| PipelineError::Message(format!("nvdec av1 create failed: {e}")))?;
        Ok(Self {
            decoder,
            require_shared_output: false,
        })
    }

    pub fn new_hevc() -> Result<Self, PipelineError> {
        let decoder = mrd_decode_nvdec::NvdecDecoder::new_hevc_with_output_mode(
            mrd_decode_nvdec::NvdecOutputMode::CpuNv12,
        )
        .map_err(|e| PipelineError::Message(format!("nvdec hevc create failed: {e}")))?;
        Ok(Self {
            decoder,
            require_shared_output: false,
        })
    }

    pub fn new_hevc_d3d11_shared() -> Result<Self, PipelineError> {
        let mut decoder = mrd_decode_nvdec::NvdecDecoder::new_hevc_with_output_mode(
            mrd_decode_nvdec::NvdecOutputMode::CpuNv12,
        )
        .map_err(|e| {
            PipelineError::Message(format!("nvdec hevc d3d11 shared create failed: {e}"))
        })?;
        #[cfg(windows)]
        decoder.enable_shared_texture(true);
        #[cfg(not(windows))]
        {
            return Err(PipelineError::Message(
                "nvdec hevc d3d11 shared output is only available on Windows".to_string(),
            ));
        }
        Ok(Self {
            decoder,
            require_shared_output: true,
        })
    }

    pub fn new_hevc_main10() -> Result<Self, PipelineError> {
        let decoder = mrd_decode_nvdec::NvdecDecoder::new_hevc_main10_with_output_mode(
            mrd_decode_nvdec::NvdecOutputMode::CpuNv12,
        )
        .map_err(|e| PipelineError::Message(format!("nvdec hevc main10 create failed: {e}")))?;
        Ok(Self {
            decoder,
            require_shared_output: false,
        })
    }

    pub fn new_hevc_main10_d3d11_shared() -> Result<Self, PipelineError> {
        let mut decoder = mrd_decode_nvdec::NvdecDecoder::new_hevc_main10_with_output_mode(
            mrd_decode_nvdec::NvdecOutputMode::CpuNv12,
        )
        .map_err(|e| {
            PipelineError::Message(format!("nvdec hevc main10 d3d11 shared create failed: {e}"))
        })?;
        #[cfg(windows)]
        decoder.enable_shared_texture(true);
        #[cfg(not(windows))]
        {
            return Err(PipelineError::Message(
                "nvdec hevc main10 d3d11 shared output is only available on Windows".to_string(),
            ));
        }
        Ok(Self {
            decoder,
            require_shared_output: true,
        })
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy)]
enum LinuxGstCodec {
    H264,
    Hevc,
    HevcMain10,
}

#[cfg(target_os = "linux")]
impl LinuxGstCodec {
    fn parser_element(self) -> &'static str {
        match self {
            Self::H264 => "h264parse",
            Self::Hevc | Self::HevcMain10 => "h265parse",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::Hevc => "HEVC",
            Self::HevcMain10 => "HEVC Main10",
        }
    }

    fn parse_dimensions(self, access_unit: &[u8]) -> Result<Option<(usize, usize)>, PipelineError> {
        match self {
            Self::H264 => parse_h264_dimensions(access_unit),
            Self::Hevc | Self::HevcMain10 => parse_hevc_dimensions(access_unit),
        }
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy)]
struct LinuxGstBackend {
    label: &'static str,
    required_elements: &'static [&'static str],
    pipeline_elements: &'static [&'static str],
}

#[cfg(target_os = "linux")]
const LINUX_GST_H264_BACKENDS: &[LinuxGstBackend] = &[
    LinuxGstBackend {
        label: "GStreamer VA H.264 decoder",
        required_elements: &["vah264dec", "vapostproc"],
        pipeline_elements: &["vah264dec", "!", "vapostproc"],
    },
    LinuxGstBackend {
        label: "GStreamer VA-API H.264 decoder",
        required_elements: &["vaapih264dec", "vaapipostproc"],
        pipeline_elements: &["vaapih264dec", "!", "vaapipostproc"],
    },
    LinuxGstBackend {
        label: "GStreamer NVIDIA H.264 decoder",
        required_elements: &["nvh264dec", "cudadownload"],
        pipeline_elements: &["nvh264dec", "!", "cudadownload"],
    },
];

#[cfg(target_os = "linux")]
const LINUX_GST_HEVC_BACKENDS: &[LinuxGstBackend] = &[
    LinuxGstBackend {
        label: "GStreamer VA HEVC decoder",
        required_elements: &["vah265dec", "vapostproc"],
        pipeline_elements: &["vah265dec", "!", "vapostproc"],
    },
    LinuxGstBackend {
        label: "GStreamer VA-API HEVC decoder",
        required_elements: &["vaapih265dec", "vaapipostproc"],
        pipeline_elements: &["vaapih265dec", "!", "vaapipostproc"],
    },
    LinuxGstBackend {
        label: "GStreamer NVIDIA HEVC decoder",
        required_elements: &["nvh265dec", "cudadownload"],
        pipeline_elements: &["nvh265dec", "!", "cudadownload"],
    },
];

#[cfg(target_os = "linux")]
fn select_linux_gst_backend(codec: LinuxGstCodec) -> Result<LinuxGstBackend, PipelineError> {
    if Command::new("gst-inspect-1.0")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_err()
    {
        return Err(PipelineError::Message(
            "GStreamer runtime is missing: gst-inspect-1.0 was not found".to_string(),
        ));
    }

    let backends = match codec {
        LinuxGstCodec::H264 => LINUX_GST_H264_BACKENDS,
        LinuxGstCodec::Hevc | LinuxGstCodec::HevcMain10 => LINUX_GST_HEVC_BACKENDS,
    };

    backends
        .iter()
        .copied()
        .find(|backend| {
            backend
                .required_elements
                .iter()
                .all(|element| gst_element_available(element))
        })
        .ok_or_else(|| {
            PipelineError::Message(format!(
                "No GStreamer hardware {} decoder was found",
                codec.label()
            ))
        })
}

#[cfg(target_os = "linux")]
fn gst_element_available(element: &str) -> bool {
    Command::new("gst-inspect-1.0")
        .arg(element)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
pub struct LinuxGstDecoder {
    codec: LinuxGstCodec,
    backend: LinuxGstBackend,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    frame_rx: Option<mpsc::Receiver<Vec<u8>>>,
    dimensions: Option<(usize, usize)>,
    pending_stream: Vec<u8>,
    decoded_frames: Vec<CoreDecodedFrame>,
    frame_index: u64,
}

#[cfg(target_os = "linux")]
impl LinuxGstDecoder {
    fn new(codec: LinuxGstCodec) -> Result<Self, PipelineError> {
        Ok(Self {
            codec,
            backend: select_linux_gst_backend(codec)?,
            child: None,
            stdin: None,
            frame_rx: None,
            dimensions: None,
            pending_stream: Vec::new(),
            decoded_frames: Vec::new(),
            frame_index: 0,
        })
    }

    fn start_pipeline(&mut self, width: usize, height: usize) -> Result<(), PipelineError> {
        if self.child.is_some() {
            return Ok(());
        }

        let frame_size = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(3))
            .ok_or_else(|| PipelineError::Message("decoded RGB frame size overflow".to_string()))?;

        let mut args = vec!["-q", "fdsrc", "fd=0", "!", self.codec.parser_element(), "!"];
        args.extend_from_slice(self.backend.pipeline_elements);
        args.extend_from_slice(&[
            "!",
            "videoconvert",
            "!",
            "video/x-raw,format=RGB",
            "!",
            "fdsink",
            "fd=1",
            "sync=false",
        ]);

        let mut child = Command::new("gst-launch-1.0")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                PipelineError::Message(format!(
                    "spawn GStreamer {} decoder failed ({}): {error}",
                    self.codec.label(),
                    self.backend.label
                ))
            })?;

        let stdout = child.stdout.take().ok_or_else(|| {
            PipelineError::Message("GStreamer decoder stdout pipe was not created".to_string())
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            PipelineError::Message("GStreamer decoder stdin pipe was not created".to_string())
        })?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || read_raw_rgb_frames(stdout, frame_size, tx));

        self.stdin = Some(stdin);
        self.frame_rx = Some(rx);
        self.child = Some(child);

        Ok(())
    }

    fn write_access_unit(&mut self, access_unit: &[u8]) -> Result<(), PipelineError> {
        let stdin = self.stdin.as_mut().ok_or_else(|| {
            PipelineError::Message("GStreamer decoder stdin is not available".to_string())
        })?;
        stdin
            .write_all(access_unit)
            .and_then(|()| stdin.flush())
            .map_err(|error| {
                PipelineError::Message(format!(
                    "write {} access unit to GStreamer decoder failed: {error}",
                    self.codec.label()
                ))
            })
    }

    fn collect_frames(&mut self) {
        let Some((width, height)) = self.dimensions else {
            return;
        };
        let Some(rx) = self.frame_rx.as_ref() else {
            return;
        };

        while let Ok(rgb) = rx.try_recv() {
            let timestamp_us = self.frame_index.saturating_mul(16_667);
            self.frame_index = self.frame_index.saturating_add(1);
            self.decoded_frames.push(CoreDecodedFrame::from_cpu_rgb24(
                width,
                height,
                timestamp_us,
                rgb,
            ));
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for LinuxGstDecoder {
    fn drop(&mut self) {
        drop(self.stdin.take());
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(target_os = "linux")]
impl VideoDecoder for LinuxGstDecoder {
    fn push_access_unit(&mut self, access_unit: &[u8]) -> Result<(), PipelineError> {
        if access_unit.is_empty() {
            return Ok(());
        }

        if let Some((width, height)) = self.codec.parse_dimensions(access_unit)? {
            if let Some((current_width, current_height)) = self.dimensions {
                if (width, height) != (current_width, current_height) {
                    return Err(PipelineError::Message(format!(
                        "Linux {} decoder does not support stream size changes yet: {current_width}x{current_height} -> {width}x{height}",
                        self.codec.label()
                    )));
                }
            } else {
                self.dimensions = Some((width, height));
            }
        }

        if self.child.is_none() {
            self.pending_stream.extend_from_slice(access_unit);
            if let Some((width, height)) = self.dimensions {
                self.start_pipeline(width, height)?;
                let pending = std::mem::take(&mut self.pending_stream);
                self.write_access_unit(&pending)?;
            } else if self.pending_stream.len() > 8 * 1024 * 1024 {
                return Err(PipelineError::Message(format!(
                    "Linux {} decoder is waiting for an SPS NAL to discover stream dimensions",
                    self.codec.label()
                )));
            }
        } else {
            self.write_access_unit(access_unit)?;
        }

        self.collect_frames();
        Ok(())
    }

    fn drain_decoded_frames(&mut self) -> Vec<CoreDecodedFrame> {
        self.collect_frames();
        std::mem::take(&mut self.decoded_frames)
    }
}

#[cfg(target_os = "linux")]
fn read_raw_rgb_frames(
    mut stdout: std::process::ChildStdout,
    frame_size: usize,
    tx: mpsc::Sender<Vec<u8>>,
) {
    loop {
        let mut frame = vec![0_u8; frame_size];
        match stdout.read_exact(&mut frame) {
            Ok(()) => {
                if tx.send(frame).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

impl H264SoftwareDecoder {
    pub fn new() -> Result<Self, PipelineError> {
        Ok(Self {
            decoder: OpenH264Decoder::new()
                .map_err(|e| PipelineError::Message(format!("openh264 init failed: {e}")))?,
            decoded_frames: Vec::new(),
        })
    }
}

impl VideoDecoder for H264SoftwareDecoder {
    fn push_access_unit(&mut self, access_unit: &[u8]) -> Result<(), PipelineError> {
        let decoded_frame = match self.decoder.decode(access_unit) {
            Ok(Some(decoded)) => Some(decoded_yuv_to_rgb_frame(&decoded, 0)),
            Ok(None) => None,
            Err(e) => {
                return Err(PipelineError::Message(format!(
                    "openh264 decode failed: {e}"
                )))
            }
        };

        if let Some(frame) = decoded_frame {
            self.decoded_frames.push(frame);
        }

        Ok(())
    }

    fn drain_decoded_frames(&mut self) -> Vec<CoreDecodedFrame> {
        std::mem::take(&mut self.decoded_frames)
    }
}

fn decoded_yuv_to_rgb_frame(decoded: &DecodedYUV<'_>, timestamp_us: u64) -> CoreDecodedFrame {
    let (width, height) = decoded.dimensions();
    let mut rgb = vec![0_u8; width * height * 3];
    decoded.write_rgb8(&mut rgb);
    CoreDecodedFrame::from_cpu_rgb24(width, height, timestamp_us, rgb)
}

fn parse_h264_dimensions(access_unit: &[u8]) -> Result<Option<(usize, usize)>, PipelineError> {
    for nal in annex_b_nals(access_unit) {
        if nal.is_empty() {
            continue;
        }
        if nal[0] & 0x1f == 7 {
            return parse_sps_dimensions(&nal[1..]).map(Some);
        }
    }

    Ok(None)
}

fn parse_hevc_dimensions(access_unit: &[u8]) -> Result<Option<(usize, usize)>, PipelineError> {
    for nal in annex_b_nals(access_unit) {
        if nal.len() < 3 {
            continue;
        }

        let nal_unit_type = (nal[0] >> 1) & 0x3f;
        if nal_unit_type == 33 {
            return parse_hevc_sps_dimensions(&nal[2..]).map(Some);
        }
    }

    Ok(None)
}

fn annex_b_nals(bytes: &[u8]) -> Vec<&[u8]> {
    let mut nals = Vec::new();
    let mut offset = 0;

    while let Some((start, start_code_len)) = find_start_code(bytes, offset) {
        let nal_start = start + start_code_len;
        let next_start = find_start_code(bytes, nal_start)
            .map(|(next, _)| next)
            .unwrap_or(bytes.len());
        if nal_start < next_start {
            let mut nal_end = next_start;
            while nal_end > nal_start && bytes[nal_end - 1] == 0 {
                nal_end -= 1;
            }
            nals.push(&bytes[nal_start..nal_end]);
        }
        offset = next_start;
    }

    nals
}

fn find_start_code(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut i = from;
    while i + 3 <= bytes.len() {
        if bytes[i] == 0 && bytes[i + 1] == 0 {
            if bytes[i + 2] == 1 {
                return Some((i, 3));
            }
            if i + 4 <= bytes.len() && bytes[i + 2] == 0 && bytes[i + 3] == 1 {
                return Some((i, 4));
            }
        }
        i += 1;
    }
    None
}

fn parse_sps_dimensions(sps: &[u8]) -> Result<(usize, usize), PipelineError> {
    let rbsp = remove_emulation_prevention_bytes(sps);
    let mut bits = BitReader::new(&rbsp);

    let profile_idc = bits
        .read_bits(8)
        .ok_or_else(|| PipelineError::Message("invalid H.264 SPS: missing profile".to_string()))?;
    bits.read_bits(8).ok_or_else(|| {
        PipelineError::Message("invalid H.264 SPS: missing constraint flags".to_string())
    })?;
    bits.read_bits(8)
        .ok_or_else(|| PipelineError::Message("invalid H.264 SPS: missing level".to_string()))?;
    bits.read_ue().ok_or_else(|| {
        PipelineError::Message("invalid H.264 SPS: missing sequence id".to_string())
    })?;

    let mut chroma_format_idc = 1_u32;
    if matches!(
        profile_idc,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        chroma_format_idc = bits.read_ue().ok_or_else(|| {
            PipelineError::Message("invalid H.264 SPS: missing chroma format".to_string())
        })?;
        if chroma_format_idc == 3 {
            bits.read_bit().ok_or_else(|| {
                PipelineError::Message(
                    "invalid H.264 SPS: missing separate colour plane flag".to_string(),
                )
            })?;
        }
        bits.read_ue().ok_or_else(|| {
            PipelineError::Message("invalid H.264 SPS: missing bit depth luma".to_string())
        })?;
        bits.read_ue().ok_or_else(|| {
            PipelineError::Message("invalid H.264 SPS: missing bit depth chroma".to_string())
        })?;
        bits.read_bit().ok_or_else(|| {
            PipelineError::Message("invalid H.264 SPS: missing qpprime flag".to_string())
        })?;
        if bits.read_bit().ok_or_else(|| {
            PipelineError::Message("invalid H.264 SPS: missing scaling matrix flag".to_string())
        })? {
            let scaling_list_count = if chroma_format_idc != 3 { 8 } else { 12 };
            for index in 0..scaling_list_count {
                if bits.read_bit().ok_or_else(|| {
                    PipelineError::Message(
                        "invalid H.264 SPS: missing scaling list flag".to_string(),
                    )
                })? {
                    skip_scaling_list(&mut bits, if index < 6 { 16 } else { 64 })?;
                }
            }
        }
    }

    bits.read_ue().ok_or_else(|| {
        PipelineError::Message("invalid H.264 SPS: missing max frame num".to_string())
    })?;
    let pic_order_cnt_type = bits.read_ue().ok_or_else(|| {
        PipelineError::Message("invalid H.264 SPS: missing pic order count type".to_string())
    })?;
    if pic_order_cnt_type == 0 {
        bits.read_ue().ok_or_else(|| {
            PipelineError::Message("invalid H.264 SPS: missing pic order cnt lsb".to_string())
        })?;
    } else if pic_order_cnt_type == 1 {
        bits.read_bit().ok_or_else(|| {
            PipelineError::Message("invalid H.264 SPS: missing delta pic order flag".to_string())
        })?;
        bits.read_se().ok_or_else(|| {
            PipelineError::Message("invalid H.264 SPS: missing offset non-ref".to_string())
        })?;
        bits.read_se().ok_or_else(|| {
            PipelineError::Message("invalid H.264 SPS: missing offset top-bottom".to_string())
        })?;
        let cycle_count = bits.read_ue().ok_or_else(|| {
            PipelineError::Message("invalid H.264 SPS: missing ref frame cycle count".to_string())
        })?;
        for _ in 0..cycle_count {
            bits.read_se().ok_or_else(|| {
                PipelineError::Message("invalid H.264 SPS: missing ref frame offset".to_string())
            })?;
        }
    }

    bits.read_ue().ok_or_else(|| {
        PipelineError::Message("invalid H.264 SPS: missing max ref frames".to_string())
    })?;
    bits.read_bit().ok_or_else(|| {
        PipelineError::Message("invalid H.264 SPS: missing gaps flag".to_string())
    })?;
    let pic_width_in_mbs_minus1 = bits
        .read_ue()
        .ok_or_else(|| PipelineError::Message("invalid H.264 SPS: missing width".to_string()))?;
    let pic_height_in_map_units_minus1 = bits
        .read_ue()
        .ok_or_else(|| PipelineError::Message("invalid H.264 SPS: missing height".to_string()))?;
    let frame_mbs_only_flag = bits.read_bit().ok_or_else(|| {
        PipelineError::Message("invalid H.264 SPS: missing frame mbs flag".to_string())
    })?;
    if !frame_mbs_only_flag {
        bits.read_bit().ok_or_else(|| {
            PipelineError::Message("invalid H.264 SPS: missing mb adaptive flag".to_string())
        })?;
    }
    bits.read_bit().ok_or_else(|| {
        PipelineError::Message("invalid H.264 SPS: missing direct 8x8 flag".to_string())
    })?;

    let mut crop_left = 0_u32;
    let mut crop_right = 0_u32;
    let mut crop_top = 0_u32;
    let mut crop_bottom = 0_u32;
    if bits
        .read_bit()
        .ok_or_else(|| PipelineError::Message("invalid H.264 SPS: missing crop flag".to_string()))?
    {
        crop_left = bits
            .read_ue()
            .ok_or_else(|| PipelineError::Message("invalid H.264 SPS: crop left".to_string()))?;
        crop_right = bits
            .read_ue()
            .ok_or_else(|| PipelineError::Message("invalid H.264 SPS: crop right".to_string()))?;
        crop_top = bits
            .read_ue()
            .ok_or_else(|| PipelineError::Message("invalid H.264 SPS: crop top".to_string()))?;
        crop_bottom = bits
            .read_ue()
            .ok_or_else(|| PipelineError::Message("invalid H.264 SPS: crop bottom".to_string()))?;
    }

    let frame_mbs_factor = if frame_mbs_only_flag { 1 } else { 2 };
    let width = (pic_width_in_mbs_minus1 + 1)
        .checked_mul(16)
        .ok_or_else(|| PipelineError::Message("invalid H.264 SPS: width overflow".to_string()))?;
    let height = (pic_height_in_map_units_minus1 + 1)
        .checked_mul(16)
        .and_then(|value| value.checked_mul(frame_mbs_factor))
        .ok_or_else(|| PipelineError::Message("invalid H.264 SPS: height overflow".to_string()))?;

    let (crop_unit_x, crop_unit_y) = crop_units(chroma_format_idc, frame_mbs_only_flag);
    let crop_width = (crop_left + crop_right)
        .checked_mul(crop_unit_x)
        .ok_or_else(|| {
            PipelineError::Message("invalid H.264 SPS: crop width overflow".to_string())
        })?;
    let crop_height = (crop_top + crop_bottom)
        .checked_mul(crop_unit_y)
        .ok_or_else(|| {
            PipelineError::Message("invalid H.264 SPS: crop height overflow".to_string())
        })?;
    let display_width = width.checked_sub(crop_width).ok_or_else(|| {
        PipelineError::Message("invalid H.264 SPS: crop exceeds width".to_string())
    })?;
    let display_height = height.checked_sub(crop_height).ok_or_else(|| {
        PipelineError::Message("invalid H.264 SPS: crop exceeds height".to_string())
    })?;

    if display_width == 0 || display_height == 0 {
        return Err(PipelineError::Message(
            "invalid H.264 SPS: zero-sized frame".to_string(),
        ));
    }

    Ok((display_width as usize, display_height as usize))
}

fn parse_hevc_sps_dimensions(sps: &[u8]) -> Result<(usize, usize), PipelineError> {
    let rbsp = remove_emulation_prevention_bytes(sps);
    let mut bits = BitReader::new(&rbsp);

    bits.read_bits(4)
        .ok_or_else(|| PipelineError::Message("invalid HEVC SPS: missing VPS id".to_string()))?;
    let max_sub_layers_minus1 = bits.read_bits(3).ok_or_else(|| {
        PipelineError::Message("invalid HEVC SPS: missing sub-layer count".to_string())
    })? as usize;
    bits.read_bit().ok_or_else(|| {
        PipelineError::Message("invalid HEVC SPS: missing temporal nesting flag".to_string())
    })?;

    skip_hevc_profile_tier_level(&mut bits, max_sub_layers_minus1)?;

    bits.read_ue().ok_or_else(|| {
        PipelineError::Message("invalid HEVC SPS: missing sequence id".to_string())
    })?;
    let chroma_format_idc = bits.read_ue().ok_or_else(|| {
        PipelineError::Message("invalid HEVC SPS: missing chroma format".to_string())
    })?;
    let separate_colour_plane = if chroma_format_idc == 3 {
        bits.read_bit().ok_or_else(|| {
            PipelineError::Message(
                "invalid HEVC SPS: missing separate colour plane flag".to_string(),
            )
        })?
    } else {
        false
    };
    let pic_width_in_luma_samples = bits
        .read_ue()
        .ok_or_else(|| PipelineError::Message("invalid HEVC SPS: missing width".to_string()))?;
    let pic_height_in_luma_samples = bits
        .read_ue()
        .ok_or_else(|| PipelineError::Message("invalid HEVC SPS: missing height".to_string()))?;

    let mut crop_left = 0_u32;
    let mut crop_right = 0_u32;
    let mut crop_top = 0_u32;
    let mut crop_bottom = 0_u32;
    if bits.read_bit().ok_or_else(|| {
        PipelineError::Message("invalid HEVC SPS: missing conformance window flag".to_string())
    })? {
        crop_left = bits
            .read_ue()
            .ok_or_else(|| PipelineError::Message("invalid HEVC SPS: crop left".to_string()))?;
        crop_right = bits
            .read_ue()
            .ok_or_else(|| PipelineError::Message("invalid HEVC SPS: crop right".to_string()))?;
        crop_top = bits
            .read_ue()
            .ok_or_else(|| PipelineError::Message("invalid HEVC SPS: crop top".to_string()))?;
        crop_bottom = bits
            .read_ue()
            .ok_or_else(|| PipelineError::Message("invalid HEVC SPS: crop bottom".to_string()))?;
    }

    let (crop_unit_x, crop_unit_y) = hevc_crop_units(chroma_format_idc, separate_colour_plane);
    let crop_width = (crop_left + crop_right)
        .checked_mul(crop_unit_x)
        .ok_or_else(|| {
            PipelineError::Message("invalid HEVC SPS: crop width overflow".to_string())
        })?;
    let crop_height = (crop_top + crop_bottom)
        .checked_mul(crop_unit_y)
        .ok_or_else(|| {
            PipelineError::Message("invalid HEVC SPS: crop height overflow".to_string())
        })?;
    let display_width = pic_width_in_luma_samples
        .checked_sub(crop_width)
        .ok_or_else(|| {
            PipelineError::Message("invalid HEVC SPS: crop exceeds width".to_string())
        })?;
    let display_height = pic_height_in_luma_samples
        .checked_sub(crop_height)
        .ok_or_else(|| {
            PipelineError::Message("invalid HEVC SPS: crop exceeds height".to_string())
        })?;

    if display_width == 0 || display_height == 0 {
        return Err(PipelineError::Message(
            "invalid HEVC SPS: zero-sized frame".to_string(),
        ));
    }

    Ok((display_width as usize, display_height as usize))
}

fn skip_hevc_profile_tier_level(
    bits: &mut BitReader<'_>,
    max_sub_layers_minus1: usize,
) -> Result<(), PipelineError> {
    skip_hevc_profile_info(bits)?;
    bits.skip_bits(8).ok_or_else(|| {
        PipelineError::Message("invalid HEVC SPS: missing general level".to_string())
    })?;

    let mut sub_layer_profile_present = vec![false; max_sub_layers_minus1];
    let mut sub_layer_level_present = vec![false; max_sub_layers_minus1];
    for index in 0..max_sub_layers_minus1 {
        sub_layer_profile_present[index] = bits.read_bit().ok_or_else(|| {
            PipelineError::Message("invalid HEVC SPS: sub-layer profile flag".to_string())
        })?;
        sub_layer_level_present[index] = bits.read_bit().ok_or_else(|| {
            PipelineError::Message("invalid HEVC SPS: sub-layer level flag".to_string())
        })?;
    }

    if max_sub_layers_minus1 > 0 {
        for _ in max_sub_layers_minus1..8 {
            bits.skip_bits(2).ok_or_else(|| {
                PipelineError::Message("invalid HEVC SPS: reserved sub-layer bits".to_string())
            })?;
        }
    }

    for index in 0..max_sub_layers_minus1 {
        if sub_layer_profile_present[index] {
            skip_hevc_profile_info(bits)?;
        }
        if sub_layer_level_present[index] {
            bits.skip_bits(8).ok_or_else(|| {
                PipelineError::Message("invalid HEVC SPS: sub-layer level".to_string())
            })?;
        }
    }

    Ok(())
}

fn skip_hevc_profile_info(bits: &mut BitReader<'_>) -> Result<(), PipelineError> {
    bits.skip_bits(2 + 1 + 5 + 32 + 4 + 44)
        .ok_or_else(|| PipelineError::Message("invalid HEVC SPS: profile tier".to_string()))
}

fn hevc_crop_units(chroma_format_idc: u32, separate_colour_plane: bool) -> (u32, u32) {
    let chroma_array_type = if separate_colour_plane {
        0
    } else {
        chroma_format_idc
    };
    match chroma_array_type {
        1 => (2, 2),
        2 => (2, 1),
        _ => (1, 1),
    }
}

fn remove_emulation_prevention_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut zero_count = 0_u8;
    for &byte in bytes {
        if zero_count == 2 && byte == 0x03 {
            zero_count = 0;
            continue;
        }
        out.push(byte);
        if byte == 0 {
            zero_count = zero_count.saturating_add(1).min(2);
        } else {
            zero_count = 0;
        }
    }
    out
}

fn crop_units(chroma_format_idc: u32, frame_mbs_only_flag: bool) -> (u32, u32) {
    let frame_factor = if frame_mbs_only_flag { 1 } else { 2 };
    match chroma_format_idc {
        0 => (1, frame_factor),
        1 => (2, 2 * frame_factor),
        2 => (2, frame_factor),
        3 => (1, frame_factor),
        _ => (1, frame_factor),
    }
}

fn skip_scaling_list(bits: &mut BitReader<'_>, size: usize) -> Result<(), PipelineError> {
    let mut last_scale = 8_i32;
    let mut next_scale = 8_i32;
    for _ in 0..size {
        if next_scale != 0 {
            let delta_scale = bits.read_se().ok_or_else(|| {
                PipelineError::Message("invalid H.264 SPS: scaling list delta".to_string())
            })?;
            next_scale = (last_scale + delta_scale + 256) % 256;
        }
        if next_scale != 0 {
            last_scale = next_scale;
        }
    }
    Ok(())
}

struct BitReader<'a> {
    bytes: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, bit_pos: 0 }
    }

    fn read_bit(&mut self) -> Option<bool> {
        let byte = *self.bytes.get(self.bit_pos / 8)?;
        let shift = 7 - (self.bit_pos % 8);
        self.bit_pos += 1;
        Some(((byte >> shift) & 1) != 0)
    }

    fn read_bits(&mut self, count: usize) -> Option<u32> {
        let mut value = 0_u32;
        for _ in 0..count {
            value = (value << 1) | u32::from(self.read_bit()?);
        }
        Some(value)
    }

    fn skip_bits(&mut self, count: usize) -> Option<()> {
        for _ in 0..count {
            self.read_bit()?;
        }
        Some(())
    }

    fn read_ue(&mut self) -> Option<u32> {
        let mut leading_zero_bits = 0_u32;
        while !self.read_bit()? {
            leading_zero_bits += 1;
            if leading_zero_bits > 31 {
                return None;
            }
        }
        if leading_zero_bits == 0 {
            return Some(0);
        }
        let suffix = self.read_bits(leading_zero_bits as usize)?;
        Some((1_u32 << leading_zero_bits) - 1 + suffix)
    }

    fn read_se(&mut self) -> Option<i32> {
        let code_num = self.read_ue()? as i32;
        let magnitude = (code_num + 1) / 2;
        if code_num % 2 == 0 {
            Some(-magnitude)
        } else {
            Some(magnitude)
        }
    }
}

impl VideoDecoder for NvdecVideoDecoder {
    fn push_access_unit(&mut self, access_unit: &[u8]) -> Result<(), PipelineError> {
        self.decoder.push_access_unit(access_unit).map_err(|e| {
            PipelineError::Message(format!(
                "nvdec decode failed: {e}; diagnostics={:?}",
                self.decoder.diagnostics()
            ))
        })
    }

    fn drain_decoded_frames(&mut self) -> Vec<CoreDecodedFrame> {
        use mrd_decode_nvdec::NvdecDecodedFrameData;
        let require_shared_output = self.require_shared_output;
        self.decoder
            .drain_decoded_frames()
            .into_iter()
            .filter_map(|frame| match frame.data {
                NvdecDecodedFrameData::CpuRgb24(data) => (!require_shared_output)
                    .then(|| CoreDecodedFrame::from_cpu_rgb24(frame.width, frame.height, 0, data)),
                NvdecDecodedFrameData::CpuNv12 { data, pitch } => {
                    (!require_shared_output).then(|| {
                        CoreDecodedFrame::from_cpu_nv12(frame.width, frame.height, 0, pitch, data)
                    })
                }
                NvdecDecodedFrameData::CpuP010 { data, pitch } => {
                    (!require_shared_output).then(|| {
                        CoreDecodedFrame::from_cpu_p010(frame.width, frame.height, 0, pitch, data)
                    })
                }
                #[cfg(windows)]
                NvdecDecodedFrameData::D3D11SharedNv12 {
                    shared_handle_y,
                    shared_handle_uv,
                    width: _,
                    height: _,
                } => Some(CoreDecodedFrame::from_d3d11_shared_nv12(
                    frame.width,
                    frame.height,
                    0,
                    shared_handle_y,
                    shared_handle_uv,
                )),
                #[cfg(windows)]
                NvdecDecodedFrameData::D3D11SharedP010 {
                    shared_handle_y,
                    shared_handle_uv,
                    width: _,
                    height: _,
                } => Some(CoreDecodedFrame::from_d3d11_shared_p010(
                    frame.width,
                    frame.height,
                    0,
                    shared_handle_y,
                    shared_handle_uv,
                )),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_encode_openh264::OpenH264Encoder;
    use mrd_pipeline_core::{CapturedFrame, FramePixelFormat, VideoEncoder};

    #[test]
    fn parses_openh264_sps_dimensions() {
        let width = 64;
        let height = 48;
        let mut encoder = OpenH264Encoder::new(width, height, 30).expect("create encoder");
        let frame = CapturedFrame::from_cpu(
            width,
            height,
            FramePixelFormat::Bgra32,
            0,
            vec![127; width * height * 4],
        );

        let access_units = encoder.encode(&frame).expect("encode frame");
        let dimensions =
            parse_h264_dimensions(&access_units[0].bytes).expect("parse H.264 dimensions");

        assert_eq!(dimensions, Some((width, height)));
    }

    #[test]
    fn exposes_hevc_d3d11_shared_nvdec_descriptor() {
        let descriptor = available_decoder_descriptors()
            .into_iter()
            .find(|descriptor| descriptor.id == "nvdec_hevc_d3d11_shared")
            .expect("HEVC D3D11 shared NVDEC descriptor");

        assert_eq!(descriptor.codec, CodecKind::Hevc);
        assert_eq!(descriptor.output_formats, D3D11_TEXTURE_OUTPUTS);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn exposes_linux_hardware_decode_descriptors_on_linux() {
        let ids = available_decoder_descriptors()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>();

        assert!(ids.contains(&"linux_h264"));
        assert!(ids.contains(&"linux_hevc"));
        assert!(ids.contains(&"linux_hevc_main10"));
    }
}
