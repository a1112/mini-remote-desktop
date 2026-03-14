use std::ffi::{c_int, c_void};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NvdecRuntimeProbe {
    pub backend: &'static str,
    pub summary: String,
    pub checked_items: Vec<&'static str>,
    pub capability_probes: Vec<NvdecCapabilityProbe>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NvdecDecodedFrame {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NvdecDiagnostics {
    pub last_stage: Option<String>,
    pub last_api: Option<String>,
    pub last_code: Option<i32>,
    pub last_error_name: Option<String>,
    pub last_error_description: Option<String>,
    pub last_picture_index: Option<i32>,
    pub decode_calls: usize,
    pub display_calls: usize,
    pub recreate_count: usize,
    pub last_recreate_reason: Option<String>,
    pub active_coded_width: Option<u32>,
    pub active_coded_height: Option<u32>,
    pub active_display_width: Option<u32>,
    pub active_display_height: Option<u32>,
    pub last_sequence_coded_width: Option<u32>,
    pub last_sequence_coded_height: Option<u32>,
    pub last_sequence_display_width: Option<u32>,
    pub last_sequence_display_height: Option<u32>,
    pub last_sequence_bit_depth_minus8: Option<u8>,
    pub last_sequence_chroma_format: Option<i32>,
    pub last_sequence_decision: Option<String>,
    pub last_recreate_from_coded_width: Option<u32>,
    pub last_recreate_from_coded_height: Option<u32>,
    pub last_recreate_to_coded_width: Option<u32>,
    pub last_recreate_to_coded_height: Option<u32>,
    pub last_decode_status_phase: Option<String>,
    pub last_decode_status_raw: Option<i32>,
    pub last_decode_status_description: Option<String>,
    pub last_reconfigure_attempted: bool,
    pub last_reconfigure_result: Option<String>,
    pub last_reconfigure_from_coded_width: Option<u32>,
    pub last_reconfigure_from_coded_height: Option<u32>,
    pub last_reconfigure_to_coded_width: Option<u32>,
    pub last_reconfigure_to_coded_height: Option<u32>,
    pub reconfigure_fallback_used: bool,
    pub last_support_codec: Option<String>,
    pub last_support_bit_depth_minus8: Option<u8>,
    pub last_support_chroma_format: Option<i32>,
    pub last_support_decision: Option<String>,
    pub last_support_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NvdecCapabilityProbe {
    pub codec: String,
    pub bit_depth_minus8: u8,
    pub chroma_format: i32,
    pub runtime_supported: bool,
    pub runtime_reason: String,
    pub wired_supported: bool,
    pub wired_reason: String,
}

pub struct NvdecDecoder {
    runtime: NvdecRuntimeProbe,
    #[cfg(windows)]
    session: imp::NvdecSession,
}

unsafe impl Send for NvdecDecoder {}

impl NvdecDecoder {
    pub fn new() -> Result<Self, String> {
        #[cfg(windows)]
        {
            let session = imp::NvdecSession::new()?;
            let runtime = NvdecRuntimeProbe {
                backend: "windows-nvdec",
                summary: "nvdec runtime libraries and core exports are present".to_string(),
                checked_items: vec![
                    "nvcuda.dll",
                    "nvcuvid.dll",
                    "cuInit",
                    "cuDeviceGetCount",
                    "cuvidGetDecoderCaps",
                    "cuvidCreateDecoder",
                    "cuvidDestroyDecoder",
                    "cuvidCreateVideoParser",
                    "cuvidParseVideoData",
                ],
                capability_probes: Vec::new(),
            };
            Ok(Self { runtime, session })
        }

        #[cfg(not(windows))]
        {
            Err("Windows-only nvdec backend is unavailable on this platform".to_string())
        }
    }

    pub fn runtime(&self) -> &NvdecRuntimeProbe {
        &self.runtime
    }

    pub fn diagnostics(&self) -> NvdecDiagnostics {
        #[cfg(windows)]
        {
            self.session.diagnostics()
        }

        #[cfg(not(windows))]
        {
            NvdecDiagnostics::default()
        }
    }

    pub fn push_access_unit(&mut self, access_unit: &[u8]) -> Result<(), String> {
        #[cfg(windows)]
        {
            self.session.push_access_unit(access_unit)
        }

        #[cfg(not(windows))]
        {
            let _ = access_unit;
            Err("Windows-only nvdec backend is unavailable on this platform".to_string())
        }
    }

    pub fn drain_decoded_frames(&mut self) -> Vec<NvdecDecodedFrame> {
        #[cfg(windows)]
        {
            self.session.drain_decoded_frames()
        }

        #[cfg(not(windows))]
        {
            Vec::new()
        }
    }
}

pub fn probe_h264_available() -> Result<(), String> {
    let probe = probe_capability("h264", 0, 1)?;
    if probe.runtime_supported && probe.wired_supported {
        Ok(())
    } else if !probe.runtime_supported {
        Err(probe.runtime_reason)
    } else {
        Err(probe.wired_reason)
    }
}

pub fn probe_hevc_available() -> Result<(), String> {
    let probe = probe_capability("hevc", 0, 1)?;
    if probe.runtime_supported && probe.wired_supported {
        Ok(())
    } else if !probe.runtime_supported {
        Err(probe.runtime_reason)
    } else {
        Err(probe.wired_reason)
    }
}

pub fn probe_hevc_main10_available() -> Result<(), String> {
    let probe = probe_capability("hevc", 2, 1)?;
    if probe.runtime_supported && probe.wired_supported {
        Ok(())
    } else if !probe.runtime_supported {
        Err(probe.runtime_reason)
    } else {
        Err(probe.wired_reason)
    }
}

pub fn probe_runtime() -> NvdecRuntimeProbe {
    let capability_probes = runtime_capability_probes();
    match NvdecDecoder::new() {
        Ok(mut decoder) => {
            decoder.runtime.capability_probes = capability_probes;
            decoder.runtime().clone()
        }
        Err(error) => NvdecRuntimeProbe {
            backend: "windows-nvdec",
            summary: error,
            checked_items: vec!["nvcuda.dll", "nvcuvid.dll"],
            capability_probes,
        },
    }
}

fn runtime_capability_probes() -> Vec<NvdecCapabilityProbe> {
    [
        ("h264", 0, 1, "H264 decode path wired"),
        ("hevc", 0, 1, "HEVC decode path not wired yet"),
        ("hevc", 2, 1, "HEVC Main10 decode path not wired yet"),
    ]
    .into_iter()
    .map(
        |(codec, bit_depth_minus8, chroma_format, fallback_wired_reason)| match probe_capability(
            codec,
            bit_depth_minus8,
            chroma_format,
        ) {
            Ok(probe) => probe,
            Err(error) => NvdecCapabilityProbe {
                codec: codec.to_string(),
                bit_depth_minus8,
                chroma_format,
                runtime_supported: false,
                runtime_reason: error,
                wired_supported: false,
                wired_reason: fallback_wired_reason.to_string(),
            },
        },
    )
    .collect()
}

fn probe_capability(
    codec: &'static str,
    bit_depth_minus8: u8,
    chroma_format: i32,
) -> Result<NvdecCapabilityProbe, String> {
    #[cfg(windows)]
    {
        imp::probe_capability(codec, bit_depth_minus8, chroma_format)
    }

    #[cfg(not(windows))]
    {
        let _ = (codec, bit_depth_minus8, chroma_format);
        Err("Windows-only nvdec backend is unavailable on this platform".to_string())
    }
}

#[cfg(windows)]
mod imp {
    #![allow(non_snake_case)]

    use super::{c_int, c_void, NvdecCapabilityProbe, NvdecDecodedFrame, NvdecDiagnostics};
    use std::{mem, ptr};
    use windows::core::PCSTR;
    use windows::Win32::Foundation::{FreeLibrary, HMODULE};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

    type CUresult = i32;
    type CUdevice = c_int;
    type CUcontext = *mut c_void;
    type CUvideodecoder = *mut c_void;
    type CUvideoparser = *mut c_void;
    type CUvideoctxlock = *mut c_void;
    type CUdeviceptr = u64;
    type CUvideotimestamp = i64;

    const CUDA_SUCCESS: CUresult = 0;
    const CUDA_ERROR_NO_DEVICE: CUresult = 100;
    const CUDA_VIDEO_CODEC_H264: i32 = 4;
    const CUDA_VIDEO_CODEC_HEVC: i32 = 6;
    const CUDA_VIDEO_CHROMA_420: i32 = 1;
    const CUDA_VIDEO_CREATE_PREFER_CUVID: u32 = 0x04;
    const CUDA_VIDEO_SURFACE_NV12: i32 = 0;
    const CUDA_VIDEO_DEINTERLACE_WEAVE: i32 = 0;
    const CUVID_PKT_ENDOFPICTURE: u32 = 0x08;

    type CuInitFn = unsafe extern "system" fn(u32) -> CUresult;
    type CuDeviceGetCountFn = unsafe extern "system" fn(*mut c_int) -> CUresult;
    type CuDeviceGetFn = unsafe extern "system" fn(*mut CUdevice, c_int) -> CUresult;
    type CuCtxCreateFn = unsafe extern "system" fn(*mut CUcontext, u32, CUdevice) -> CUresult;
    type CuCtxDestroyFn = unsafe extern "system" fn(CUcontext) -> CUresult;
    type CuGetErrorNameFn = unsafe extern "system" fn(CUresult, *mut *const i8) -> CUresult;
    type CuGetErrorStringFn = unsafe extern "system" fn(CUresult, *mut *const i8) -> CUresult;
    type CuMemcpyDtoHFn = unsafe extern "system" fn(*mut c_void, CUdeviceptr, usize) -> CUresult;
    type CuvidGetDecoderCapsFn = unsafe extern "system" fn(*mut CUVIDDECODECAPS) -> CUresult;
    type CuvidCreateDecoderFn =
        unsafe extern "system" fn(*mut CUvideodecoder, *mut CUVIDDECODECREATEINFO) -> CUresult;
    type CuvidDestroyDecoderFn = unsafe extern "system" fn(CUvideodecoder) -> CUresult;
    type CuvidDecodePictureFn =
        unsafe extern "system" fn(CUvideodecoder, *mut CUVIDPICPARAMS) -> CUresult;
    type CuvidGetDecodeStatusFn =
        unsafe extern "system" fn(CUvideodecoder, c_int, *mut CUVIDGETDECODESTATUS) -> CUresult;
    type CuvidReconfigureDecoderFn =
        unsafe extern "system" fn(CUvideodecoder, *mut CUVIDRECONFIGUREDECODERINFO) -> CUresult;
    type CuvidCreateVideoParserFn =
        unsafe extern "system" fn(*mut CUvideoparser, *mut CUVIDPARSERPARAMS) -> CUresult;
    type CuvidParseVideoDataFn =
        unsafe extern "system" fn(CUvideoparser, *mut CUVIDSOURCEDATAPACKET) -> CUresult;
    type CuvidDestroyVideoParserFn = unsafe extern "system" fn(CUvideoparser) -> CUresult;
    type CuvidMapVideoFrameFn = unsafe extern "system" fn(
        CUvideodecoder,
        c_int,
        *mut u64,
        *mut u32,
        *mut CUVIDPROCPARAMS,
    ) -> CUresult;
    type CuvidUnmapVideoFrameFn = unsafe extern "system" fn(CUvideodecoder, u64) -> CUresult;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct DisplayRect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct AspectRatio {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct VideoSignalDescription {
        flags: u8,
        color_primaries: u8,
        transfer_characteristics: u8,
        matrix_coefficients: u8,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CUVIDEOFORMAT {
        codec: i32,
        frame_rate: FrameRate,
        progressive_sequence: u8,
        bit_depth_luma_minus8: u8,
        bit_depth_chroma_minus8: u8,
        min_num_decode_surfaces: u8,
        coded_width: u32,
        coded_height: u32,
        display_area: DisplayRect,
        chroma_format: i32,
        bitrate: u32,
        display_aspect_ratio: AspectRatio,
        video_signal_description: VideoSignalDescription,
        seqhdr_data_length: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FrameRate {
        numerator: u32,
        denominator: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ShortRect {
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CUVIDDECODECREATEINFO {
        ulWidth: u32,
        ulHeight: u32,
        ulNumDecodeSurfaces: u32,
        CodecType: i32,
        ChromaFormat: i32,
        ulCreationFlags: u32,
        bitDepthMinus8: u32,
        ulIntraDecodeOnly: u32,
        ulMaxWidth: u32,
        ulMaxHeight: u32,
        Reserved1: u32,
        display_area: ShortRect,
        OutputFormat: i32,
        DeinterlaceMode: i32,
        ulTargetWidth: u32,
        ulTargetHeight: u32,
        ulNumOutputSurfaces: u32,
        vidLock: CUvideoctxlock,
        target_rect: ShortRect,
        enableHistogram: u32,
        Reserved2: [u32; 4],
    }

    #[repr(C)]
    struct CUVIDPICPARAMS {
        _opaque: [u8; 0],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CUVIDPARSERDISPINFO {
        picture_index: c_int,
        progressive_frame: c_int,
        top_field_first: c_int,
        repeat_first_field: c_int,
        timestamp: CUvideotimestamp,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CUVIDPARSERPARAMS {
        CodecType: i32,
        ulMaxNumDecodeSurfaces: u32,
        ulClockRate: u32,
        ulErrorThreshold: u32,
        ulMaxDisplayDelay: u32,
        bitfields: u32,
        uReserved1: [u32; 4],
        pUserData: *mut c_void,
        pfnSequenceCallback:
            Option<unsafe extern "system" fn(*mut c_void, *mut CUVIDEOFORMAT) -> c_int>,
        pfnDecodePicture:
            Option<unsafe extern "system" fn(*mut c_void, *mut CUVIDPICPARAMS) -> c_int>,
        pfnDisplayPicture:
            Option<unsafe extern "system" fn(*mut c_void, *mut CUVIDPARSERDISPINFO) -> c_int>,
        pfnGetOperatingPoint: Option<unsafe extern "system" fn(*mut c_void, *mut c_void) -> c_int>,
        pvReserved2: [*mut c_void; 6],
        pExtVideoInfo: *mut c_void,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CUVIDSOURCEDATAPACKET {
        flags: u32,
        payload_size: u32,
        payload: *const u8,
        timestamp: CUvideotimestamp,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CUVIDPROCPARAMS {
        progressive_frame: c_int,
        second_field: c_int,
        top_field_first: c_int,
        unpaired_field: c_int,
        reserved_flags: u32,
        reserved_zero: u32,
        raw_input_dptr: u64,
        raw_input_pitch: u32,
        raw_input_format: u32,
        raw_output_dptr: u64,
        raw_output_pitch: u32,
        raw_output_format: u32,
        raw_output_surface: u32,
        reserved1: u32,
        Reserved: [u32; 48],
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CUVIDDECODECAPS {
        eCodecType: i32,
        eChromaFormat: i32,
        nBitDepthMinus8: u32,
        reserved1: [u32; 3],
        bIsSupported: u8,
        nNumNVDECs: u8,
        nOutputFormatMask: u16,
        nMaxWidth: u32,
        nMaxHeight: u32,
        nMaxMBCount: u32,
        nMinWidth: u16,
        nMinHeight: u16,
        bIsHistogramSupported: u8,
        nCounterBitDepth: u8,
        nMaxHistogramBins: u16,
        reserved3: [u32; 10],
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CUVIDGETDECODESTATUS {
        decodeStatus: i32,
        reserved: [u32; 31],
        pReserved: [*mut c_void; 8],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CUVIDRECONFIGUREDECODERINFO {
        ulWidth: u32,
        ulHeight: u32,
        ulTargetWidth: u32,
        ulTargetHeight: u32,
        ulNumDecodeSurfaces: u32,
        display_area: ShortRect,
        target_rect: ShortRect,
        Reserved1: [u32; 8],
        Reserved2: [*mut c_void; 6],
    }

    struct LoadedModule(HMODULE);

    impl LoadedModule {
        fn load(name: &'static [u8]) -> Result<Self, String> {
            let module = unsafe { LoadLibraryA(PCSTR(name.as_ptr())) }.map_err(|_| {
                format!(
                    "failed to load {}",
                    String::from_utf8_lossy(&name[..name.len().saturating_sub(1)])
                )
            })?;
            Ok(Self(module))
        }

        fn load_symbol<T>(&self, name: &'static [u8]) -> Result<T, String>
        where
            T: Copy,
        {
            let proc =
                unsafe { GetProcAddress(self.0, PCSTR(name.as_ptr())) }.ok_or_else(|| {
                    format!(
                        "missing export {}",
                        String::from_utf8_lossy(&name[..name.len().saturating_sub(1)])
                    )
                })?;
            let ptr = proc as *const ();
            Ok(unsafe { mem::transmute_copy(&ptr) })
        }
    }

    impl Drop for LoadedModule {
        fn drop(&mut self) {
            unsafe {
                let _ = FreeLibrary(self.0);
            }
        }
    }

    struct CudaApi {
        _module: LoadedModule,
        cu_init: CuInitFn,
        cu_device_get_count: CuDeviceGetCountFn,
        cu_device_get: CuDeviceGetFn,
        cu_ctx_create: CuCtxCreateFn,
        cu_ctx_destroy: CuCtxDestroyFn,
        cu_get_error_name: Option<CuGetErrorNameFn>,
        cu_get_error_string: Option<CuGetErrorStringFn>,
        cu_memcpy_dtoh: CuMemcpyDtoHFn,
    }

    impl CudaApi {
        fn load() -> Result<Self, String> {
            let module = LoadedModule::load(b"nvcuda.dll\0")?;
            Ok(Self {
                cu_init: module.load_symbol(b"cuInit\0")?,
                cu_device_get_count: module.load_symbol(b"cuDeviceGetCount\0")?,
                cu_device_get: module.load_symbol(b"cuDeviceGet\0")?,
                cu_ctx_create: module.load_symbol(b"cuCtxCreate_v2\0")?,
                cu_ctx_destroy: module.load_symbol(b"cuCtxDestroy_v2\0")?,
                cu_get_error_name: module.load_symbol(b"cuGetErrorName\0").ok(),
                cu_get_error_string: module.load_symbol(b"cuGetErrorString\0").ok(),
                cu_memcpy_dtoh: module.load_symbol(b"cuMemcpyDtoH_v2\0")?,
                _module: module,
            })
        }
    }

    struct CuvidApi {
        _module: LoadedModule,
        cuvid_get_decoder_caps: Option<CuvidGetDecoderCapsFn>,
        cuvid_create_decoder: CuvidCreateDecoderFn,
        cuvid_destroy_decoder: CuvidDestroyDecoderFn,
        cuvid_decode_picture: CuvidDecodePictureFn,
        cuvid_get_decode_status: Option<CuvidGetDecodeStatusFn>,
        cuvid_reconfigure_decoder: Option<CuvidReconfigureDecoderFn>,
        cuvid_create_video_parser: CuvidCreateVideoParserFn,
        cuvid_parse_video_data: CuvidParseVideoDataFn,
        cuvid_destroy_video_parser: CuvidDestroyVideoParserFn,
        cuvid_map_video_frame: CuvidMapVideoFrameFn,
        cuvid_unmap_video_frame: CuvidUnmapVideoFrameFn,
    }

    impl CuvidApi {
        fn load() -> Result<Self, String> {
            let module = LoadedModule::load(b"nvcuvid.dll\0")?;
            Ok(Self {
                cuvid_get_decoder_caps: module.load_symbol(b"cuvidGetDecoderCaps\0").ok(),
                cuvid_create_decoder: module.load_symbol(b"cuvidCreateDecoder\0")?,
                cuvid_destroy_decoder: module.load_symbol(b"cuvidDestroyDecoder\0")?,
                cuvid_decode_picture: module.load_symbol(b"cuvidDecodePicture\0")?,
                cuvid_get_decode_status: module.load_symbol(b"cuvidGetDecodeStatus\0").ok(),
                cuvid_reconfigure_decoder: module.load_symbol(b"cuvidReconfigureDecoder\0").ok(),
                cuvid_create_video_parser: module.load_symbol(b"cuvidCreateVideoParser\0")?,
                cuvid_parse_video_data: module.load_symbol(b"cuvidParseVideoData\0")?,
                cuvid_destroy_video_parser: module.load_symbol(b"cuvidDestroyVideoParser\0")?,
                cuvid_map_video_frame: module.load_symbol(b"cuvidMapVideoFrame64\0")?,
                cuvid_unmap_video_frame: module.load_symbol(b"cuvidUnmapVideoFrame64\0")?,
                _module: module,
            })
        }
    }

    pub struct NvdecSession {
        _cuda: CudaApi,
        _cuvid: CuvidApi,
        context: CUcontext,
        parser: CUvideoparser,
        callback_state: Box<CallbackState>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct SequenceFormat {
        codec: i32,
        coded_width: u32,
        coded_height: u32,
        display_width: u32,
        display_height: u32,
        chroma_format: i32,
        bit_depth_minus8: u8,
        min_decode_surfaces: u8,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct DecoderConfig {
        codec: i32,
        coded_width: u32,
        coded_height: u32,
        display_width: u32,
        display_height: u32,
        chroma_format: i32,
        bit_depth_minus8: u8,
        decode_surfaces: u32,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum SequenceChangeDecision {
        Reuse,
        Recreate(&'static str),
        Unsupported(&'static str),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ReconfigureDecision {
        Attempt,
        SkipUnsupported,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum NvdecCodec {
        H264,
        Hevc,
        Unknown(i32),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct NvdecSupportRequest {
        codec: NvdecCodec,
        bit_depth_minus8: u8,
        chroma_format: i32,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum NvdecSupportDecision {
        Supported,
        Unsupported(&'static str),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct NvdecCapabilityRequest {
        codec: NvdecCodec,
        bit_depth_minus8: u8,
        chroma_format: i32,
    }

    struct CallbackState {
        cuvid_create_decoder: CuvidCreateDecoderFn,
        cuvid_destroy_decoder: CuvidDestroyDecoderFn,
        cuvid_decode_picture: CuvidDecodePictureFn,
        cuvid_get_decode_status: Option<CuvidGetDecodeStatusFn>,
        cuvid_reconfigure_decoder: Option<CuvidReconfigureDecoderFn>,
        cuvid_map_video_frame: CuvidMapVideoFrameFn,
        cuvid_unmap_video_frame: CuvidUnmapVideoFrameFn,
        cu_get_error_name: Option<CuGetErrorNameFn>,
        cu_get_error_string: Option<CuGetErrorStringFn>,
        cu_memcpy_dtoh: CuMemcpyDtoHFn,
        decoder: CUvideodecoder,
        decoder_config: Option<DecoderConfig>,
        sequence_width: u32,
        sequence_height: u32,
        diagnostics: NvdecDiagnostics,
        last_error: Option<String>,
        frames: Vec<NvdecDecodedFrame>,
    }

    impl NvdecSession {
        pub fn new() -> Result<Self, String> {
            let cuda = CudaApi::load()?;
            let cuvid = CuvidApi::load()?;

            unsafe {
                cuda_ok(&cuda, (cuda.cu_init)(0), "init", "cuInit")?;
            }

            let mut count = 0;
            unsafe {
                cuda_ok(
                    &cuda,
                    (cuda.cu_device_get_count)(&mut count),
                    "init",
                    "cuDeviceGetCount",
                )?;
            }
            if count <= 0 {
                return Err("cuDeviceGetCount reported no CUDA devices".to_string());
            }

            if let Some(get_caps) = cuvid.cuvid_get_decoder_caps {
                let mut caps = CUVIDDECODECAPS {
                    eCodecType: CUDA_VIDEO_CODEC_H264,
                    eChromaFormat: CUDA_VIDEO_CHROMA_420,
                    nBitDepthMinus8: 0,
                    ..Default::default()
                };
                let caps_result = unsafe { get_caps(&mut caps) };
                if caps_result == CUDA_SUCCESS && caps.bIsSupported == 0 {
                    return Err("cuvidGetDecoderCaps reported H264 NVDEC unsupported".to_string());
                }
            }

            let mut device = 0;
            unsafe {
                cuda_ok(
                    &cuda,
                    (cuda.cu_device_get)(&mut device, 0),
                    "init",
                    "cuDeviceGet",
                )?;
            }

            let mut context = ptr::null_mut();
            unsafe {
                cuda_ok(
                    &cuda,
                    (cuda.cu_ctx_create)(&mut context, 0, device),
                    "init",
                    "cuCtxCreate_v2",
                )?;
            }

            let mut callback_state = Box::new(CallbackState {
                cuvid_create_decoder: cuvid.cuvid_create_decoder,
                cuvid_destroy_decoder: cuvid.cuvid_destroy_decoder,
                cuvid_decode_picture: cuvid.cuvid_decode_picture,
                cuvid_get_decode_status: cuvid.cuvid_get_decode_status,
                cuvid_reconfigure_decoder: cuvid.cuvid_reconfigure_decoder,
                cuvid_map_video_frame: cuvid.cuvid_map_video_frame,
                cuvid_unmap_video_frame: cuvid.cuvid_unmap_video_frame,
                cu_get_error_name: cuda.cu_get_error_name,
                cu_get_error_string: cuda.cu_get_error_string,
                cu_memcpy_dtoh: cuda.cu_memcpy_dtoh,
                decoder: ptr::null_mut(),
                decoder_config: None,
                sequence_width: 0,
                sequence_height: 0,
                diagnostics: NvdecDiagnostics::default(),
                last_error: None,
                frames: Vec::new(),
            });

            let mut parser = ptr::null_mut();
            let mut params = CUVIDPARSERPARAMS {
                CodecType: CUDA_VIDEO_CODEC_H264,
                ulMaxNumDecodeSurfaces: 8,
                ulClockRate: 10_000_000,
                ulErrorThreshold: 0,
                ulMaxDisplayDelay: 0,
                bitfields: 0,
                uReserved1: [0; 4],
                pUserData: callback_state.as_mut() as *mut CallbackState as *mut c_void,
                pfnSequenceCallback: Some(sequence_callback),
                pfnDecodePicture: Some(decode_callback),
                pfnDisplayPicture: Some(display_callback),
                pfnGetOperatingPoint: None,
                pvReserved2: [ptr::null_mut(); 6],
                pExtVideoInfo: ptr::null_mut(),
            };
            unsafe {
                cuda_ok(
                    &cuda,
                    (cuvid.cuvid_create_video_parser)(&mut parser, &mut params),
                    "init",
                    "cuvidCreateVideoParser",
                )?;
            }

            Ok(Self {
                _cuda: cuda,
                _cuvid: cuvid,
                context,
                parser,
                callback_state,
            })
        }

        pub fn push_access_unit(&mut self, access_unit: &[u8]) -> Result<(), String> {
            if !looks_like_annexb(access_unit) {
                self.callback_state.clear_access_unit_state();
                let message =
                    "nvdec input validation failed: expected H264 Annex-B access unit".to_string();
                self.callback_state.last_error = Some(message.clone());
                self.callback_state.diagnostics.last_stage = Some("input".to_string());
                self.callback_state.diagnostics.last_api = Some("push_access_unit".to_string());
                self.callback_state.diagnostics.last_error_description =
                    Some("expected H264 Annex-B start code".to_string());
                return Err(message);
            }

            self.callback_state.clear_access_unit_state();

            let payload_size = u32::try_from(access_unit.len())
                .map_err(|_| "access unit too large for NVDEC packet".to_string())?;
            let mut packet = CUVIDSOURCEDATAPACKET {
                flags: CUVID_PKT_ENDOFPICTURE,
                payload_size,
                payload: access_unit.as_ptr(),
                timestamp: 0,
            };

            unsafe {
                cuda_ok(
                    &self._cuda,
                    (self._cuvid.cuvid_parse_video_data)(self.parser, &mut packet),
                    "parse",
                    "cuvidParseVideoData",
                )?;
            }

            if let Some(error) = self.callback_state.last_error.take() {
                return Err(error);
            }

            if self.callback_state.sequence_width == 0 || self.callback_state.sequence_height == 0 {
                return Err(
                    "nvdec parse failed at parser-state: parser did not report sequence information"
                        .to_string(),
                );
            }

            if self.callback_state.diagnostics.decode_calls == 0 {
                return Err(
                    "nvdec decode failed at callback-state: parser completed without decode activity"
                        .to_string(),
                );
            }

            Ok(())
        }

        pub fn drain_decoded_frames(&mut self) -> Vec<NvdecDecodedFrame> {
            mem::take(&mut self.callback_state.frames)
        }

        pub fn diagnostics(&self) -> NvdecDiagnostics {
            self.callback_state.diagnostics.clone()
        }
    }

    impl Drop for NvdecSession {
        fn drop(&mut self) {
            unsafe {
                if !self.parser.is_null() {
                    let _ = (self._cuvid.cuvid_destroy_video_parser)(self.parser);
                }
                if !self.callback_state.decoder.is_null() {
                    let _ = (self._cuvid.cuvid_destroy_decoder)(self.callback_state.decoder);
                }
                if !self.context.is_null() {
                    let _ = (self._cuda.cu_ctx_destroy)(self.context);
                }
            }
        }
    }

    impl CallbackState {
        fn clear_access_unit_state(&mut self) {
            self.last_error = None;
            self.diagnostics.last_stage = None;
            self.diagnostics.last_api = None;
            self.diagnostics.last_code = None;
            self.diagnostics.last_error_name = None;
            self.diagnostics.last_error_description = None;
            self.diagnostics.last_picture_index = None;
            self.diagnostics.decode_calls = 0;
            self.diagnostics.display_calls = 0;
            self.diagnostics.last_decode_status_phase = None;
            self.diagnostics.last_decode_status_raw = None;
            self.diagnostics.last_decode_status_description = None;
            self.diagnostics.last_reconfigure_attempted = false;
            self.diagnostics.last_reconfigure_result = None;
            self.diagnostics.last_reconfigure_from_coded_width = None;
            self.diagnostics.last_reconfigure_from_coded_height = None;
            self.diagnostics.last_reconfigure_to_coded_width = None;
            self.diagnostics.last_reconfigure_to_coded_height = None;
            self.diagnostics.reconfigure_fallback_used = false;
        }

        fn record_recreate(&mut self, reason: &'static str, config: &DecoderConfig) {
            self.diagnostics.recreate_count += 1;
            self.diagnostics.last_recreate_reason = Some(reason.to_string());
            self.diagnostics.last_sequence_decision = Some("recreate".to_string());
            self.diagnostics.last_recreate_to_coded_width = Some(config.coded_width);
            self.diagnostics.last_recreate_to_coded_height = Some(config.coded_height);
            self.diagnostics.active_coded_width = Some(config.coded_width);
            self.diagnostics.active_coded_height = Some(config.coded_height);
            self.diagnostics.active_display_width = Some(config.display_width);
            self.diagnostics.active_display_height = Some(config.display_height);
        }

        fn record_active_config(&mut self, config: &DecoderConfig) {
            self.diagnostics.active_coded_width = Some(config.coded_width);
            self.diagnostics.active_coded_height = Some(config.coded_height);
            self.diagnostics.active_display_width = Some(config.display_width);
            self.diagnostics.active_display_height = Some(config.display_height);
        }

        fn record_sequence(&mut self, sequence: &SequenceFormat) {
            self.diagnostics.last_sequence_coded_width = Some(sequence.coded_width);
            self.diagnostics.last_sequence_coded_height = Some(sequence.coded_height);
            self.diagnostics.last_sequence_display_width = Some(sequence.display_width);
            self.diagnostics.last_sequence_display_height = Some(sequence.display_height);
            self.diagnostics.last_sequence_bit_depth_minus8 = Some(sequence.bit_depth_minus8);
            self.diagnostics.last_sequence_chroma_format = Some(sequence.chroma_format);
            self.diagnostics.last_support_codec = Some(describe_codec(sequence.codec).to_string());
            self.diagnostics.last_support_bit_depth_minus8 = Some(sequence.bit_depth_minus8);
            self.diagnostics.last_support_chroma_format = Some(sequence.chroma_format);
        }

        fn record_sequence_decision(&mut self, decision: &str) {
            self.diagnostics.last_sequence_decision = Some(decision.to_string());
        }

        fn record_support_decision(&mut self, decision: &str, reason: Option<&str>) {
            self.diagnostics.last_support_decision = Some(decision.to_string());
            self.diagnostics.last_support_reason = reason.map(ToString::to_string);
        }

        fn record_recreate_from(&mut self, config: &DecoderConfig) {
            self.diagnostics.last_recreate_from_coded_width = Some(config.coded_width);
            self.diagnostics.last_recreate_from_coded_height = Some(config.coded_height);
        }

        fn record_reconfigure_attempt(&mut self, from: &DecoderConfig, to: &SequenceFormat) {
            self.diagnostics.last_reconfigure_attempted = true;
            self.diagnostics.last_reconfigure_from_coded_width = Some(from.coded_width);
            self.diagnostics.last_reconfigure_from_coded_height = Some(from.coded_height);
            self.diagnostics.last_reconfigure_to_coded_width = Some(to.coded_width);
            self.diagnostics.last_reconfigure_to_coded_height = Some(to.coded_height);
        }

        fn record_reconfigure_result(&mut self, result: String) {
            self.diagnostics.last_reconfigure_result = Some(result);
        }

        fn record_decode_status_snapshot(
            &mut self,
            phase: &'static str,
            raw: Option<i32>,
            description: String,
        ) {
            self.diagnostics.last_decode_status_phase = Some(phase.to_string());
            self.diagnostics.last_decode_status_raw = raw;
            self.diagnostics.last_decode_status_description = Some(description);
        }

        fn record_failure(
            &mut self,
            stage: &'static str,
            api: &'static str,
            code: CUresult,
            context: Option<String>,
        ) {
            let (name, description) =
                describe_cuda_error(self.cu_get_error_name, self.cu_get_error_string, code);
            self.diagnostics.last_stage = Some(stage.to_string());
            self.diagnostics.last_api = Some(api.to_string());
            self.diagnostics.last_code = Some(code);
            self.diagnostics.last_error_name = name.clone();
            self.diagnostics.last_error_description = description.clone().or(context.clone());
            self.last_error = Some(format_cuda_failure(
                stage,
                api,
                code,
                name.as_deref(),
                description.as_deref(),
                context.as_deref(),
            ));
        }
    }

    impl SequenceFormat {
        fn from_video_format(format: &CUVIDEOFORMAT) -> Self {
            Self {
                codec: format.codec,
                coded_width: format.coded_width,
                coded_height: format.coded_height,
                display_width: format.display_area.right.max(1) as u32,
                display_height: format.display_area.bottom.max(1) as u32,
                chroma_format: format.chroma_format,
                bit_depth_minus8: format.bit_depth_luma_minus8,
                min_decode_surfaces: format.min_num_decode_surfaces.max(1),
            }
        }
    }

    impl DecoderConfig {
        fn from_sequence(sequence: &SequenceFormat) -> Self {
            Self {
                codec: sequence.codec,
                coded_width: sequence.coded_width,
                coded_height: sequence.coded_height,
                display_width: sequence.display_width,
                display_height: sequence.display_height,
                chroma_format: sequence.chroma_format,
                bit_depth_minus8: sequence.bit_depth_minus8,
                decode_surfaces: u32::from(sequence.min_decode_surfaces.max(1)),
            }
        }

        fn evaluate_sequence_change(&self, next: &SequenceFormat) -> SequenceChangeDecision {
            if next.chroma_format != CUDA_VIDEO_CHROMA_420 {
                return SequenceChangeDecision::Unsupported("chroma format change");
            }

            if next.bit_depth_minus8 != 0 {
                return SequenceChangeDecision::Unsupported("bit depth change");
            }

            if self.coded_width != next.coded_width || self.coded_height != next.coded_height {
                return SequenceChangeDecision::Recreate("coded size changed");
            }

            if self.display_width != next.display_width
                || self.display_height != next.display_height
            {
                return SequenceChangeDecision::Recreate("display size changed");
            }

            if self.decode_surfaces != u32::from(next.min_decode_surfaces.max(1)) {
                return SequenceChangeDecision::Recreate("decode surface count changed");
            }

            SequenceChangeDecision::Reuse
        }

        fn evaluate_reconfigure(&self, next: &SequenceFormat) -> ReconfigureDecision {
            if next.chroma_format != CUDA_VIDEO_CHROMA_420 || next.bit_depth_minus8 != 0 {
                return ReconfigureDecision::SkipUnsupported;
            }

            if self.coded_width != next.coded_width
                || self.coded_height != next.coded_height
                || self.display_width != next.display_width
                || self.display_height != next.display_height
                || self.decode_surfaces != u32::from(next.min_decode_surfaces.max(1))
            {
                return ReconfigureDecision::Attempt;
            }

            ReconfigureDecision::SkipUnsupported
        }
    }

    unsafe extern "system" fn sequence_callback(
        user_data: *mut c_void,
        format: *mut CUVIDEOFORMAT,
    ) -> c_int {
        let state = unsafe { &mut *(user_data as *mut CallbackState) };
        let format = unsafe { &*format };
        let next_sequence = SequenceFormat::from_video_format(format);
        state.sequence_width = next_sequence.coded_width;
        state.sequence_height = next_sequence.coded_height;
        state.record_sequence(&next_sequence);
        let support_request = NvdecSupportRequest::from_sequence(&next_sequence);
        match evaluate_support(support_request) {
            NvdecSupportDecision::Supported => {
                state.record_support_decision("supported", None);
            }
            NvdecSupportDecision::Unsupported(reason) => {
                state.record_support_decision("unsupported", Some(reason));
                state.last_error = Some(format!("nvdec sequence unsupported: {reason}"));
                state.diagnostics.last_stage = Some("sequence".to_string());
                state.diagnostics.last_api = Some("support-matrix".to_string());
                state.diagnostics.last_error_description = Some(reason.to_string());
                return 0;
            }
        }

        match state.decoder_config.clone() {
            None => {
                state.record_sequence_decision("create");
                if let Err(error) = create_decoder_for_sequence(state, &next_sequence) {
                    state.last_error = Some(error);
                    return 0;
                }
            }
            Some(current) => match current.evaluate_sequence_change(&next_sequence) {
                SequenceChangeDecision::Reuse => {
                    state.record_sequence_decision("reuse");
                    state.record_active_config(&current);
                }
                SequenceChangeDecision::Recreate(reason) => {
                    match current.evaluate_reconfigure(&next_sequence) {
                        ReconfigureDecision::Attempt => {
                            match try_reconfigure_decoder(state, &current, &next_sequence, reason) {
                                Ok(true) => {
                                    state.record_sequence_decision("reconfigure");
                                }
                                Ok(false) => {
                                    state.diagnostics.reconfigure_fallback_used = true;
                                    state.record_recreate_from(&current);
                                    if let Err(error) =
                                        destroy_active_decoder(state, "recreate", reason)
                                    {
                                        state.last_error = Some(error);
                                        return 0;
                                    }
                                    if let Err(error) =
                                        create_decoder_for_sequence(state, &next_sequence)
                                    {
                                        state.last_error = Some(error);
                                        return 0;
                                    }
                                    if let Some(config) = state.decoder_config.clone() {
                                        state.record_sequence_decision("recreate");
                                        state.record_recreate(reason, &config);
                                    }
                                }
                                Err(error) => {
                                    state.last_error = Some(error);
                                    return 0;
                                }
                            }
                        }
                        ReconfigureDecision::SkipUnsupported => {
                            state.record_recreate_from(&current);
                            if let Err(error) = destroy_active_decoder(state, "recreate", reason) {
                                state.last_error = Some(error);
                                return 0;
                            }
                            if let Err(error) = create_decoder_for_sequence(state, &next_sequence) {
                                state.last_error = Some(error);
                                return 0;
                            }
                            if let Some(config) = state.decoder_config.clone() {
                                state.record_sequence_decision("recreate");
                                state.record_recreate(reason, &config);
                            }
                        }
                    }
                }
                SequenceChangeDecision::Unsupported(reason) => {
                    state.record_sequence_decision("unsupported");
                    state.last_error = Some(format!("nvdec sequence change unsupported: {reason}"));
                    state.diagnostics.last_stage = Some("sequence".to_string());
                    state.diagnostics.last_api = Some("sequence_callback".to_string());
                    state.diagnostics.last_error_description = Some(reason.to_string());
                    return 0;
                }
            },
        }

        next_sequence.min_decode_surfaces.max(1) as c_int
    }

    unsafe extern "system" fn decode_callback(
        user_data: *mut c_void,
        pic_params: *mut CUVIDPICPARAMS,
    ) -> c_int {
        let state = unsafe { &mut *(user_data as *mut CallbackState) };
        if state.decoder.is_null() {
            state.last_error =
                Some("nvdec decode failed at callback-state: decoder was not created".to_string());
            state.diagnostics.last_stage = Some("decode".to_string());
            state.diagnostics.last_api = Some("cuvidDecodePicture".to_string());
            return 0;
        }
        let result = unsafe { (state.cuvid_decode_picture)(state.decoder, pic_params) };
        if result != CUDA_SUCCESS {
            state.record_failure("decode", "cuvidDecodePicture", result, None);
            return 0;
        }
        state.diagnostics.decode_calls += 1;
        state.record_decode_status_snapshot(
            "decode",
            None,
            "cuvidGetDecodeStatus skipped in decode phase: picture index unavailable".to_string(),
        );
        1
    }

    unsafe extern "system" fn display_callback(
        user_data: *mut c_void,
        disp_info: *mut CUVIDPARSERDISPINFO,
    ) -> c_int {
        let state = unsafe { &mut *(user_data as *mut CallbackState) };
        if disp_info.is_null() {
            return 1;
        }
        let disp_info = unsafe { &*disp_info };
        state.diagnostics.display_calls += 1;
        state.diagnostics.last_picture_index = Some(disp_info.picture_index);
        record_decode_status_for_picture(state, "display", disp_info.picture_index);
        if state.sequence_width == 0 || state.sequence_height == 0 {
            return 1;
        }

        let mut dev_ptr = 0_u64;
        let mut pitch = 0_u32;
        let mut proc_params = CUVIDPROCPARAMS {
            progressive_frame: disp_info.progressive_frame,
            second_field: 0,
            top_field_first: disp_info.top_field_first,
            unpaired_field: 0,
            reserved_flags: 0,
            reserved_zero: 0,
            raw_input_dptr: 0,
            raw_input_pitch: 0,
            raw_input_format: 0,
            raw_output_dptr: 0,
            raw_output_pitch: 0,
            raw_output_format: 0,
            raw_output_surface: 0,
            reserved1: 0,
            Reserved: [0; 48],
        };

        let map_result = unsafe {
            (state.cuvid_map_video_frame)(
                state.decoder,
                disp_info.picture_index,
                &mut dev_ptr,
                &mut pitch,
                &mut proc_params,
            )
        };
        if map_result != CUDA_SUCCESS {
            state.record_failure("map", "cuvidMapVideoFrame64", map_result, None);
            return 0;
        }

        let width = state.sequence_width as usize;
        let height = state.sequence_height as usize;
        let y_plane_bytes = pitch as usize * height;
        let uv_plane_bytes = pitch as usize * (height / 2);
        let total = y_plane_bytes + uv_plane_bytes;
        let mut nv12 = vec![0_u8; total];
        let copy_result =
            unsafe { (state.cu_memcpy_dtoh)(nv12.as_mut_ptr() as *mut c_void, dev_ptr, total) };
        let unmap_result = unsafe { (state.cuvid_unmap_video_frame)(state.decoder, dev_ptr) };
        if copy_result != CUDA_SUCCESS {
            state.record_failure("copy", "cuMemcpyDtoH_v2", copy_result, None);
            return 0;
        }
        if unmap_result != CUDA_SUCCESS {
            state.record_failure("unmap", "cuvidUnmapVideoFrame64", unmap_result, None);
            return 0;
        }

        let rgb = nv12_to_rgb(&nv12, width, height, pitch as usize);
        state.frames.push(NvdecDecodedFrame {
            width,
            height,
            data: rgb,
        });
        1
    }

    fn cuda_ok(
        cuda: &CudaApi,
        result: CUresult,
        stage: &'static str,
        api: &'static str,
    ) -> Result<(), String> {
        match result {
            CUDA_SUCCESS => Ok(()),
            CUDA_ERROR_NO_DEVICE => Err(format!(
                "nvdec {stage} failed at {api}: no CUDA device available"
            )),
            other => {
                let (name, description) =
                    describe_cuda_error(cuda.cu_get_error_name, cuda.cu_get_error_string, other);
                Err(format_cuda_failure(
                    stage,
                    api,
                    other,
                    name.as_deref(),
                    description.as_deref(),
                    None,
                ))
            }
        }
    }

    fn describe_cuda_error(
        get_error_name: Option<CuGetErrorNameFn>,
        get_error_string: Option<CuGetErrorStringFn>,
        code: CUresult,
    ) -> (Option<String>, Option<String>) {
        let mut name = None;
        let mut description = None;

        if let Some(get_error_name) = get_error_name {
            let mut raw_name = ptr::null();
            let status = unsafe { get_error_name(code, &mut raw_name) };
            if status == CUDA_SUCCESS && !raw_name.is_null() {
                name = Some(c_string_ptr_to_string(raw_name));
            }
        }

        if let Some(get_error_string) = get_error_string {
            let mut raw_description = ptr::null();
            let status = unsafe { get_error_string(code, &mut raw_description) };
            if status == CUDA_SUCCESS && !raw_description.is_null() {
                description = Some(c_string_ptr_to_string(raw_description));
            }
        }

        (name, description)
    }

    fn format_cuda_failure(
        stage: &'static str,
        api: &'static str,
        code: CUresult,
        error_name: Option<&str>,
        error_description: Option<&str>,
        context: Option<&str>,
    ) -> String {
        let mut message = format!("nvdec {stage} failed at {api}: code {code}");
        if let Some(error_name) = error_name {
            message.push_str(" (");
            message.push_str(error_name);
            message.push(')');
        }
        if let Some(error_description) = error_description {
            message.push_str(": ");
            message.push_str(error_description);
        }
        if let Some(context) = context {
            message.push_str(" [");
            message.push_str(context);
            message.push(']');
        }
        message
    }

    fn c_string_ptr_to_string(ptr: *const i8) -> String {
        let bytes = unsafe { std::ffi::CStr::from_ptr(ptr) };
        bytes.to_string_lossy().into_owned()
    }

    fn record_decode_status_for_picture(
        state: &mut CallbackState,
        phase: &'static str,
        picture_index: c_int,
    ) {
        let Some(cuvid_get_decode_status) = state.cuvid_get_decode_status else {
            state.record_decode_status_snapshot(
                phase,
                None,
                "cuvidGetDecodeStatus unavailable".to_string(),
            );
            return;
        };

        let mut status = CUVIDGETDECODESTATUS::default();
        let result = unsafe { cuvid_get_decode_status(state.decoder, picture_index, &mut status) };
        if result != CUDA_SUCCESS {
            let description = format!("cuvidGetDecodeStatus failed: code {result}");
            state.record_decode_status_snapshot(phase, Some(result), description);
            return;
        }

        state.record_decode_status_snapshot(
            phase,
            Some(status.decodeStatus),
            describe_decode_status(status.decodeStatus).to_string(),
        );
    }

    fn describe_decode_status(status: i32) -> &'static str {
        match status {
            0 => "invalid",
            1 => "in-progress",
            2 => "success",
            8 => "error",
            9 => "error-concealed",
            _ => "unknown",
        }
    }

    fn destroy_active_decoder(
        state: &mut CallbackState,
        stage: &'static str,
        reason: &'static str,
    ) -> Result<(), String> {
        if state.decoder.is_null() {
            state.decoder_config = None;
            return Ok(());
        }

        let result = unsafe { (state.cuvid_destroy_decoder)(state.decoder) };
        if result != CUDA_SUCCESS {
            state.record_failure(
                stage,
                "cuvidDestroyDecoder",
                result,
                Some(reason.to_string()),
            );
            return Err(state
                .last_error
                .clone()
                .unwrap_or_else(|| "nvdec recreate failed while destroying decoder".to_string()));
        }

        state.decoder = ptr::null_mut();
        state.decoder_config = None;
        Ok(())
    }

    fn create_decoder_for_sequence(
        state: &mut CallbackState,
        sequence: &SequenceFormat,
    ) -> Result<(), String> {
        match evaluate_support(NvdecSupportRequest::from_sequence(sequence)) {
            NvdecSupportDecision::Supported => {}
            NvdecSupportDecision::Unsupported(reason) => {
                return Err(format!("nvdec sequence unsupported: {reason}"));
            }
        }

        let config = DecoderConfig::from_sequence(sequence);
        let mut create_info = CUVIDDECODECREATEINFO {
            ulWidth: config.coded_width,
            ulHeight: config.coded_height,
            ulNumDecodeSurfaces: config.decode_surfaces,
            CodecType: config.codec,
            ChromaFormat: config.chroma_format,
            ulCreationFlags: CUDA_VIDEO_CREATE_PREFER_CUVID,
            bitDepthMinus8: u32::from(config.bit_depth_minus8),
            ulIntraDecodeOnly: 0,
            ulMaxWidth: config.coded_width,
            ulMaxHeight: config.coded_height,
            Reserved1: 0,
            display_area: ShortRect {
                left: 0,
                top: 0,
                right: config.display_width as i16,
                bottom: config.display_height as i16,
            },
            OutputFormat: CUDA_VIDEO_SURFACE_NV12,
            DeinterlaceMode: CUDA_VIDEO_DEINTERLACE_WEAVE,
            ulTargetWidth: config.display_width,
            ulTargetHeight: config.display_height,
            ulNumOutputSurfaces: 2,
            vidLock: ptr::null_mut(),
            target_rect: ShortRect {
                left: 0,
                top: 0,
                right: config.display_width as i16,
                bottom: config.display_height as i16,
            },
            enableHistogram: 0,
            Reserved2: [0; 4],
        };

        let mut decoder = ptr::null_mut();
        let result = unsafe { (state.cuvid_create_decoder)(&mut decoder, &mut create_info) };
        if result != CUDA_SUCCESS {
            state.record_failure(
                "sequence",
                "cuvidCreateDecoder",
                result,
                Some(format!(
                    "width={} height={} surfaces={} chroma={} bit_depth={} target={}x{}",
                    create_info.ulWidth,
                    create_info.ulHeight,
                    create_info.ulNumDecodeSurfaces,
                    create_info.ChromaFormat,
                    create_info.bitDepthMinus8,
                    create_info.ulTargetWidth,
                    create_info.ulTargetHeight,
                )),
            );
            return Err(state
                .last_error
                .clone()
                .unwrap_or_else(|| "nvdec sequence failed at cuvidCreateDecoder".to_string()));
        }

        state.decoder = decoder;
        state.decoder_config = Some(config.clone());
        state.record_active_config(&config);
        Ok(())
    }

    fn try_reconfigure_decoder(
        state: &mut CallbackState,
        current: &DecoderConfig,
        next: &SequenceFormat,
        reason: &'static str,
    ) -> Result<bool, String> {
        state.record_reconfigure_attempt(current, next);
        let Some(cuvid_reconfigure_decoder) = state.cuvid_reconfigure_decoder else {
            state.record_reconfigure_result("unavailable".to_string());
            return Ok(false);
        };

        let mut info = CUVIDRECONFIGUREDECODERINFO {
            ulWidth: next.coded_width,
            ulHeight: next.coded_height,
            ulTargetWidth: next.display_width,
            ulTargetHeight: next.display_height,
            ulNumDecodeSurfaces: u32::from(next.min_decode_surfaces.max(1)),
            display_area: ShortRect {
                left: 0,
                top: 0,
                right: next.display_width as i16,
                bottom: next.display_height as i16,
            },
            target_rect: ShortRect {
                left: 0,
                top: 0,
                right: next.display_width as i16,
                bottom: next.display_height as i16,
            },
            Reserved1: [0; 8],
            Reserved2: [ptr::null_mut(); 6],
        };

        let result = unsafe { cuvid_reconfigure_decoder(state.decoder, &mut info) };
        if result != CUDA_SUCCESS {
            state.record_reconfigure_result(format!(
                "failed: {}",
                format_cuda_failure(
                    "reconfigure",
                    "cuvidReconfigureDecoder",
                    result,
                    None,
                    None,
                    Some(reason)
                )
            ));
            return Ok(false);
        }

        let config = DecoderConfig::from_sequence(next);
        state.decoder_config = Some(config.clone());
        state.record_active_config(&config);
        state.record_reconfigure_result("success".to_string());
        Ok(true)
    }

    impl NvdecSupportRequest {
        fn from_sequence(sequence: &SequenceFormat) -> Self {
            Self {
                codec: match sequence.codec {
                    CUDA_VIDEO_CODEC_H264 => NvdecCodec::H264,
                    CUDA_VIDEO_CODEC_HEVC => NvdecCodec::Hevc,
                    other => NvdecCodec::Unknown(other),
                },
                bit_depth_minus8: sequence.bit_depth_minus8,
                chroma_format: sequence.chroma_format,
            }
        }
    }

    impl NvdecCapabilityRequest {
        fn new(codec: &'static str, bit_depth_minus8: u8, chroma_format: i32) -> Self {
            let codec = match codec {
                "h264" => NvdecCodec::H264,
                "hevc" => NvdecCodec::Hevc,
                _ => NvdecCodec::Unknown(-1),
            };
            Self {
                codec,
                bit_depth_minus8,
                chroma_format,
            }
        }

        fn codec_name(&self) -> &'static str {
            match self.codec {
                NvdecCodec::H264 => "h264",
                NvdecCodec::Hevc => "hevc",
                NvdecCodec::Unknown(_) => "unknown",
            }
        }

        fn probe_label(&self) -> String {
            match self.codec {
                NvdecCodec::Hevc if self.bit_depth_minus8 > 0 => {
                    format!("{} main10", self.codec_name())
                }
                _ => format!(
                    "{} {}-bit chroma {}",
                    self.codec_name(),
                    self.bit_depth_minus8 + 8,
                    self.chroma_format
                ),
            }
        }

        fn codec_raw(&self) -> i32 {
            match self.codec {
                NvdecCodec::H264 => CUDA_VIDEO_CODEC_H264,
                NvdecCodec::Hevc => CUDA_VIDEO_CODEC_HEVC,
                NvdecCodec::Unknown(raw) => raw,
            }
        }

        fn to_support_request(self) -> NvdecSupportRequest {
            NvdecSupportRequest {
                codec: self.codec,
                bit_depth_minus8: self.bit_depth_minus8,
                chroma_format: self.chroma_format,
            }
        }
    }

    fn evaluate_support(request: NvdecSupportRequest) -> NvdecSupportDecision {
        match request.codec {
            NvdecCodec::H264 => {}
            NvdecCodec::Hevc => {
                if request.bit_depth_minus8 > 0 {
                    return NvdecSupportDecision::Unsupported("HEVC Main10 not wired yet");
                }
                return NvdecSupportDecision::Unsupported("HEVC not wired yet");
            }
            NvdecCodec::Unknown(_) => {
                return NvdecSupportDecision::Unsupported("unknown codec");
            }
        }

        if request.bit_depth_minus8 > 0 {
            return NvdecSupportDecision::Unsupported("10-bit not wired yet");
        }

        if request.chroma_format != CUDA_VIDEO_CHROMA_420 {
            return NvdecSupportDecision::Unsupported("unsupported chroma format");
        }

        NvdecSupportDecision::Supported
    }

    pub fn probe_capability(
        codec: &'static str,
        bit_depth_minus8: u8,
        chroma_format: i32,
    ) -> Result<NvdecCapabilityProbe, String> {
        let request = NvdecCapabilityRequest::new(codec, bit_depth_minus8, chroma_format);
        let cuda = CudaApi::load()?;
        let cuvid = CuvidApi::load()?;

        unsafe {
            cuda_ok(&cuda, (cuda.cu_init)(0), "init", "cuInit")?;
        }

        let mut count = 0;
        unsafe {
            cuda_ok(
                &cuda,
                (cuda.cu_device_get_count)(&mut count),
                "init",
                "cuDeviceGetCount",
            )?;
        }
        if count <= 0 {
            return Err("cuDeviceGetCount reported no CUDA devices".to_string());
        }

        let wired_decision = evaluate_support(request.to_support_request());
        if let Some(get_caps) = cuvid.cuvid_get_decoder_caps {
            let mut caps = CUVIDDECODECAPS {
                eCodecType: request.codec_raw(),
                eChromaFormat: request.chroma_format,
                nBitDepthMinus8: u32::from(request.bit_depth_minus8),
                ..Default::default()
            };
            let result = unsafe { get_caps(&mut caps) };
            if result != CUDA_SUCCESS {
                return Ok(NvdecCapabilityProbe {
                    codec: request.codec_name().to_string(),
                    bit_depth_minus8: request.bit_depth_minus8,
                    chroma_format: request.chroma_format,
                    runtime_supported: false,
                    runtime_reason: format!(
                        "{} runtime probe failed at cuvidGetDecoderCaps",
                        request.probe_label()
                    ),
                    wired_supported: matches!(wired_decision, NvdecSupportDecision::Supported),
                    wired_reason: support_reason(wired_decision).to_string(),
                });
            }

            let runtime_supported = caps.bIsSupported != 0;
            let runtime_reason = if runtime_supported {
                format!(
                    "{} runtime capability reported by nvdec",
                    request.probe_label()
                )
            } else {
                format!("{} unsupported by runtime", request.probe_label())
            };
            return Ok(NvdecCapabilityProbe {
                codec: request.codec_name().to_string(),
                bit_depth_minus8: request.bit_depth_minus8,
                chroma_format: request.chroma_format,
                runtime_supported,
                runtime_reason,
                wired_supported: matches!(wired_decision, NvdecSupportDecision::Supported),
                wired_reason: support_reason(wired_decision).to_string(),
            });
        }

        Ok(NvdecCapabilityProbe {
            codec: request.codec_name().to_string(),
            bit_depth_minus8: request.bit_depth_minus8,
            chroma_format: request.chroma_format,
            runtime_supported: false,
            runtime_reason: "nvdec runtime probe unavailable: cuvidGetDecoderCaps unavailable"
                .to_string(),
            wired_supported: matches!(wired_decision, NvdecSupportDecision::Supported),
            wired_reason: support_reason(wired_decision).to_string(),
        })
    }

    fn support_reason(decision: NvdecSupportDecision) -> &'static str {
        match decision {
            NvdecSupportDecision::Supported => "wired",
            NvdecSupportDecision::Unsupported(reason) => reason,
        }
    }

    fn describe_codec(codec: i32) -> &'static str {
        match codec {
            CUDA_VIDEO_CODEC_H264 => "h264",
            CUDA_VIDEO_CODEC_HEVC => "hevc",
            _ => "unknown",
        }
    }

    fn looks_like_annexb(buf: &[u8]) -> bool {
        buf.len() >= 4
            && ((buf[0] == 0 && buf[1] == 0 && buf[2] == 1)
                || (buf[0] == 0 && buf[1] == 0 && buf[2] == 0 && buf[3] == 1))
    }

    fn nv12_to_rgb(nv12: &[u8], width: usize, height: usize, pitch: usize) -> Vec<u8> {
        let mut rgb = vec![0_u8; width * height * 3];
        let uv_base = pitch * height;

        for y in 0..height {
            for x in 0..width {
                let y_sample = nv12[y * pitch + x] as f32;
                let uv_offset = uv_base + (y / 2) * pitch + (x / 2) * 2;
                let u = nv12[uv_offset] as f32;
                let v = nv12[uv_offset + 1] as f32;

                let c = y_sample - 16.0;
                let d = u - 128.0;
                let e = v - 128.0;

                let r = (1.164 * c + 1.596 * e).round();
                let g = (1.164 * c - 0.392 * d - 0.813 * e).round();
                let b = (1.164 * c + 2.017 * d).round();

                let out = (y * width + x) * 3;
                rgb[out] = clamp_to_u8(r);
                rgb[out + 1] = clamp_to_u8(g);
                rgb[out + 2] = clamp_to_u8(b);
            }
        }

        rgb
    }

    fn clamp_to_u8(value: f32) -> u8 {
        value.clamp(0.0, 255.0) as u8
    }

    #[cfg(test)]
    mod tests {
        use super::{
            evaluate_support, DecoderConfig, NvdecCapabilityRequest, NvdecCodec,
            NvdecSupportDecision, NvdecSupportRequest, SequenceChangeDecision, SequenceFormat,
            CUDA_VIDEO_CHROMA_420, CUDA_VIDEO_CODEC_H264,
        };

        fn baseline_sequence() -> SequenceFormat {
            SequenceFormat {
                codec: CUDA_VIDEO_CODEC_H264,
                coded_width: 128,
                coded_height: 128,
                display_width: 128,
                display_height: 128,
                chroma_format: CUDA_VIDEO_CHROMA_420,
                bit_depth_minus8: 0,
                min_decode_surfaces: 8,
            }
        }

        fn baseline_config() -> DecoderConfig {
            DecoderConfig::from_sequence(&baseline_sequence())
        }

        #[test]
        fn support_matrix_accepts_h264_8bit_420() {
            let request = NvdecSupportRequest {
                codec: NvdecCodec::H264,
                bit_depth_minus8: 0,
                chroma_format: CUDA_VIDEO_CHROMA_420,
            };

            assert_eq!(evaluate_support(request), NvdecSupportDecision::Supported);
        }

        #[test]
        fn support_matrix_rejects_hevc() {
            let request = NvdecSupportRequest {
                codec: NvdecCodec::Hevc,
                bit_depth_minus8: 0,
                chroma_format: CUDA_VIDEO_CHROMA_420,
            };

            assert_eq!(
                evaluate_support(request),
                NvdecSupportDecision::Unsupported("HEVC not wired yet")
            );
        }

        #[test]
        fn support_matrix_rejects_ten_bit() {
            let request = NvdecSupportRequest {
                codec: NvdecCodec::H264,
                bit_depth_minus8: 2,
                chroma_format: CUDA_VIDEO_CHROMA_420,
            };

            assert_eq!(
                evaluate_support(request),
                NvdecSupportDecision::Unsupported("10-bit not wired yet")
            );
        }

        #[test]
        fn support_matrix_rejects_non_420() {
            let request = NvdecSupportRequest {
                codec: NvdecCodec::H264,
                bit_depth_minus8: 0,
                chroma_format: 3,
            };

            assert_eq!(
                evaluate_support(request),
                NvdecSupportDecision::Unsupported("unsupported chroma format")
            );
        }

        #[test]
        fn capability_probe_reports_hevc_as_not_wired() {
            let request = NvdecCapabilityRequest::new("hevc", 0, CUDA_VIDEO_CHROMA_420);
            let wired = evaluate_support(request.to_support_request());

            assert_eq!(
                wired,
                NvdecSupportDecision::Unsupported("HEVC not wired yet")
            );
        }

        #[test]
        fn capability_probe_reports_hevc_main10_as_not_wired() {
            let request = NvdecCapabilityRequest::new("hevc", 2, CUDA_VIDEO_CHROMA_420);
            let wired = evaluate_support(request.to_support_request());

            assert_eq!(
                wired,
                NvdecSupportDecision::Unsupported("HEVC Main10 not wired yet")
            );
        }

        #[test]
        fn recreate_decision_keeps_compatible_sequence() {
            let decision = baseline_config().evaluate_sequence_change(&baseline_sequence());
            assert_eq!(decision, SequenceChangeDecision::Reuse);
        }

        #[test]
        fn recreate_decision_recreates_on_coded_size_change() {
            let mut next = baseline_sequence();
            next.coded_width = 256;

            let decision = baseline_config().evaluate_sequence_change(&next);
            assert_eq!(
                decision,
                SequenceChangeDecision::Recreate("coded size changed")
            );
        }

        #[test]
        fn recreate_decision_recreates_on_display_size_change() {
            let mut next = baseline_sequence();
            next.display_height = 256;

            let decision = baseline_config().evaluate_sequence_change(&next);
            assert_eq!(
                decision,
                SequenceChangeDecision::Recreate("display size changed")
            );
        }

        #[test]
        fn recreate_decision_rejects_bit_depth_change() {
            let mut next = baseline_sequence();
            next.bit_depth_minus8 = 2;

            let decision = baseline_config().evaluate_sequence_change(&next);
            assert_eq!(
                decision,
                SequenceChangeDecision::Unsupported("bit depth change")
            );
        }

        #[test]
        fn recreate_decision_rejects_chroma_change() {
            let mut next = baseline_sequence();
            next.chroma_format = 3;

            let decision = baseline_config().evaluate_sequence_change(&next);
            assert_eq!(
                decision,
                SequenceChangeDecision::Unsupported("chroma format change")
            );
        }
    }
}
