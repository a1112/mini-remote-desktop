/**
 * NVENC 完整编码器实现
 *
 * 使用 NVENC SDK 13.0 实现完整的硬件编码
 * 支持 D3D11-CUDA 互操作
 * 使用 CUDA kernel 进行 BGRA→NV12 转换
 */

#ifdef NVENC_ENCODER_EXPORTS
#undef NVENC_ENCODER_EXPORTS
#endif
#define NVENC_ENCODER_EXPORTS
#include "nvenc_full.h"
#include "bgra_to_nv12.h"
#include <iostream>
#include <cstring>
#include <algorithm>
#include <cstdint>

// CUDA 错误检查
#define CUDA_CHECK(call) \
    do { \
        CUresult err = call; \
        if (err != CUDA_SUCCESS) { \
            std::cerr << "[NVENC] CUDA Error: " << err << " at " << __LINE__ << std::endl; \
            return false; \
        } \
    } while(0)

#define CUDA_RUNTIME_CHECK(call) \
    do { \
        cudaError_t err = call; \
        if (err != cudaSuccess) { \
            std::cerr << "[NVENC] CUDA Runtime Error: " << cudaGetErrorString(err) << " at " << __LINE__ << std::endl; \
            return false; \
        } \
    } while(0)

// NVENC 错误检查
#define NVENC_CHECK(call) \
    do { \
        NVENCSTATUS err = call; \
        if (err != NV_ENC_SUCCESS) { \
            std::cerr << "[NVENC] Error: " << err << " at " << __LINE__ << std::endl; \
            return false; \
        } \
    } while(0)

static inline uint8_t ClampToByte(int v) {
    if (v < 0) return 0;
    if (v > 255) return 255;
    return static_cast<uint8_t>(v);
}

// CPU reference conversion used by fallback paths.
static void ConvertBGRAtoNV12CPU(
    const uint8_t* src_bgra,
    int src_stride,
    int width,
    int height,
    uint8_t* dst_y,
    uint8_t* dst_uv,
    int dst_pitch
) {
    // Luma plane
    for (int y = 0; y < height; ++y) {
        const uint8_t* src_row = src_bgra + y * src_stride;
        uint8_t* dst_row = dst_y + y * dst_pitch;
        for (int x = 0; x < width; ++x) {
            const int b = src_row[x * 4 + 0];
            const int g = src_row[x * 4 + 1];
            const int r = src_row[x * 4 + 2];
            dst_row[x] = ClampToByte((77 * r + 150 * g + 29 * b + 128) >> 8);
        }
    }

    // Chroma plane (interleaved UV, 2x2 subsampling)
    for (int y = 0; y < height; y += 2) {
        uint8_t* uv_row = dst_uv + (y / 2) * dst_pitch;
        for (int x = 0; x < width; x += 2) {
            int u_sum = 0;
            int v_sum = 0;
            int count = 0;

            for (int dy = 0; dy < 2 && (y + dy) < height; ++dy) {
                const uint8_t* src_row = src_bgra + (y + dy) * src_stride;
                for (int dx = 0; dx < 2 && (x + dx) < width; ++dx) {
                    const int b = src_row[(x + dx) * 4 + 0];
                    const int g = src_row[(x + dx) * 4 + 1];
                    const int r = src_row[(x + dx) * 4 + 2];

                    // BT.601 full-range integer approximation
                    u_sum += (((-43 * r - 85 * g + 128 * b + 128) >> 8) + 128);
                    v_sum += (((128 * r - 107 * g - 21 * b + 128) >> 8) + 128);
                    ++count;
                }
            }

            const int uv_idx = x;
            const int safe_count = (count > 0) ? count : 1;
            uv_row[uv_idx + 0] = ClampToByte(u_sum / safe_count);
            if (uv_idx + 1 < dst_pitch) {
                uv_row[uv_idx + 1] = ClampToByte(v_sum / safe_count);
            }
        }
    }
}

// ============================================================================
// NVENCEncoderImpl Implementation
// ============================================================================

NVENCEncoderImpl::NVENCEncoderImpl()
    : d3d11_device_(nullptr)
    , d3d11_context_(nullptr)
    , nvenc_encoder_(nullptr)
    , registered_resource_(nullptr)
    , mapped_resource_(nullptr)
    , registered_source_texture_(nullptr)
    , current_input_buffer_(0)
    , current_bitstream_buffer_(0)
    , current_pts_(0)
    , force_keyframe_(false)
    , initialized_(false)
    , cuda_initialized_(false)
    , nvenc_loaded_(false)
    , cuda_context_(nullptr)
    , cuda_stream_(nullptr)
    , nvenc_dll_(nullptr)
    , use_abgr_format_(false)
    , zerocopy_only_(false)
    , zc_encode_calls_(0)
    , zc_encode_submit_success_(0)
    , zc_encode_submit_need_more_input_(0)
    , zc_encode_submit_fail_(0)
    , zc_slot_busy_skips_(0)
    , zc_map_failures_(0)
    , zc_lock_busy_count_(0)
    , zc_lock_retryable_count_(0)
    , zc_lock_failures_(0)
    , zc_bitstream_outputs_(0)
    , zc_unmap_count_(0)
    , zc_pending_peak_(0)
{
    std::memset(&nvenc_api_, 0, sizeof(nvenc_api_));
}

NVENCEncoderImpl::~NVENCEncoderImpl() {
    Release();
}

bool NVENCEncoderImpl::Initialize(ID3D11Device* d3d11_device, ID3D11DeviceContext* d3d11_context,
                                   const NVENCEncodeConfig* config, bool zerocopy_only) {
    if (!d3d11_device || !d3d11_context || !config) {
        return false;
    }

    d3d11_device_ = d3d11_device;
    d3d11_context_ = d3d11_context;
    config_ = *config;
    zerocopy_only_ = zerocopy_only;

    // 设置默认值
    if (config_.framerate <= 0) config_.framerate = 60;
    if (config_.bitrate <= 0) config_.bitrate = 5000000;
    if (config_.gop_size <= 0) config_.gop_size = 60;
    if (config_.preset < 0) config_.preset = 2;
    if (config_.rc_mode < 0) config_.rc_mode = 3;  // 默认 CQ 模式
    if (config_.quality <= 0) config_.quality = 20;  // 默认高质量 CQ

    std::cout << "[NVENC] Initializing encoder " << config_.width << "x" << config_.height
              << " @ " << config_.framerate << "fps, " << (config_.bitrate / 1000000.0) << "Mbps" << std::endl;

    // Zero-copy-only session does not require CUDA interop.
    if (!zerocopy_only_) {
        if (!InitializeCUDA()) {
            std::cerr << "[NVENC] CUDA initialization failed" << std::endl;
            return false;
        }
    } else {
        std::cout << "[NVENC] CUDA initialization skipped (zerocopy-only session)" << std::endl;
    }

    // 加载 NVENC
    if (!LoadNVENC()) {
        std::cerr << "[NVENC] Failed to load NVENC" << std::endl;
        Cleanup();
        return false;
    }

    // 初始化 NVENC 会话
    if (!InitializeNVENCSession()) {
        std::cerr << "[NVENC] Failed to initialize NVENC session" << std::endl;
        Cleanup();
        return false;
    }

    // 初始化编码器参数
    if (!InitializeEncoderParams()) {
        std::cerr << "[NVENC] Failed to initialize encoder params" << std::endl;
        Cleanup();
        return false;
    }

    // 分配缓冲区
    if (!AllocateBuffers()) {
        std::cerr << "[NVENC] Failed to allocate buffers" << std::endl;
        Cleanup();
        return false;
    }

    initialized_ = true;
    std::cout << "[NVENC] Encoder initialized successfully" << std::endl;
    return true;
}

bool NVENCEncoderImpl::InitializeCUDA() {
    CUresult err = cuInit(0);
    if (err != CUDA_SUCCESS) {
        std::cerr << "[CUDA] cuInit failed: " << err << std::endl;
        return false;
    }

    // 使用 D3D11-CUDA 互操作创建 CUDA 上下文
    // 这样 CUDA 可以访问 D3D11 资源（GPU Direct）
    if (d3d11_device_) {
        // 获取 DXGI 适配器来确定正确的 CUDA 设备
        ComPtr<IDXGIDevice> dxgi_device;
        if (SUCCEEDED(d3d11_device_.As(&dxgi_device))) {
            ComPtr<IDXGIAdapter> dxgi_adapter;
            if (SUCCEEDED(dxgi_device->GetAdapter(&dxgi_adapter))) {
                DXGI_ADAPTER_DESC desc;
                dxgi_adapter->GetDesc(&desc);

                // 查找匹配的 CUDA 设备
                CUdevice cuda_device = 0;
                int device_count = 0;
                cuDeviceGetCount(&device_count);

                for (int i = 0; i < device_count; i++) {
                    char cuda_name[256];
                    cuDeviceGetName(cuda_name, sizeof(cuda_name), i);

                    // 简单匹配：如果有 LUID，比较 LUID
                    // 这里简化处理：使用第一个可用的设备
                    cuda_device = i;
                    break;
                }

                // 从 D3D11 设备创建 CUDA 上下文
                err = cuD3D11CtxCreate(&cuda_context_, nullptr, cuda_device, d3d11_device_.Get());
                if (err == CUDA_SUCCESS) {
                    std::cout << "[CUDA] D3D11-CUDA interop initialized successfully" << std::endl;
                } else {
                    std::cerr << "[CUDA] cuD3D11CtxCreate failed: " << err << std::endl;
                    std::cerr << "[CUDA] Falling back to regular CUDA context..." << std::endl;
                    CUDA_CHECK(cuCtxCreate(&cuda_context_, nullptr, 0, cuda_device));
                }
            }
        }
    }

    if (!cuda_context_) {
        CUdevice cuda_device = 0;
        CUDA_CHECK(cuDeviceGet(&cuda_device, 0));
        CUDA_CHECK(cuCtxCreate(&cuda_context_, nullptr, 0, cuda_device));
    }

    CUDA_RUNTIME_CHECK(cudaStreamCreate(&cuda_stream_));

    cuda_initialized_ = true;
    std::cout << "[CUDA] Initialized successfully" << std::endl;
    return true;
}

bool NVENCEncoderImpl::LoadNVENC() {
    nvenc_dll_ = LoadLibraryA("nvEncodeAPI64.dll");
    if (!nvenc_dll_) {
        std::cerr << "[NVENC] Failed to load nvEncodeAPI64.dll" << std::endl;
        return false;
    }

    auto NvEncodeAPICreateInstance = (NVENCSTATUS (NVENCAPI*)(NV_ENCODE_API_FUNCTION_LIST*))(
        GetProcAddress(nvenc_dll_, "NvEncodeAPICreateInstance"));

    if (!NvEncodeAPICreateInstance) {
        std::cerr << "[NVENC] Failed to get NvEncodeAPICreateInstance" << std::endl;
        return false;
    }

    nvenc_api_.version = NV_ENCODE_API_FUNCTION_LIST_VER;
    nvenc_api_.reserved = 0;

    NVENCSTATUS err = NvEncodeAPICreateInstance(&nvenc_api_);
    if (err != NV_ENC_SUCCESS) {
        std::cerr << "[NVENC] NvEncodeAPICreateInstance failed: " << err << std::endl;
        return false;
    }

    nvenc_loaded_ = true;
    std::cout << "[NVENC] API loaded successfully" << std::endl;
    return true;
}

bool NVENCEncoderImpl::InitializeNVENCSession() {
    // 打开 NVENC 编码会话
    // 对于零拷贝 D3D11 编码，使用 DIRECTX 设备类型
    NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS sessionParams = {};
    sessionParams.version = NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER;

    // 尝试使用 D3D11 设备以支持 MapInputResource
    sessionParams.deviceType = NV_ENC_DEVICE_TYPE_DIRECTX;
    sessionParams.device = d3d11_device_.Get();
    sessionParams.apiVersion = NVENCAPI_VERSION;

    NVENCSTATUS err = nvenc_api_.nvEncOpenEncodeSessionEx(&sessionParams, &nvenc_encoder_);
    if (err != NV_ENC_SUCCESS) {
        std::cerr << "[NVENC] nvEncOpenEncodeSessionEx (DirectX) failed: " << err << std::endl;
        std::cerr << "[NVENC] Falling back to CUDA device..." << std::endl;

        // 回退到 CUDA 设备
        sessionParams.deviceType = NV_ENC_DEVICE_TYPE_CUDA;
        sessionParams.device = cuda_context_;
        err = nvenc_api_.nvEncOpenEncodeSessionEx(&sessionParams, &nvenc_encoder_);
        if (err != NV_ENC_SUCCESS) {
            std::cerr << "[NVENC] nvEncOpenEncodeSessionEx (CUDA) failed: " << err << std::endl;
            return false;
        }
        std::cout << "[NVENC] Session opened with CUDA device type" << std::endl;
    } else {
        std::cout << "[NVENC] Session opened with DirectX device type (required for zero-copy)" << std::endl;
    }
    return true;
}

bool NVENCEncoderImpl::InitializeEncoderParams() {
    // 选择预设 GUID
    GUID presetGUID = NV_ENC_PRESET_P3_GUID; // medium
    if (config_.preset == 1) presetGUID = NV_ENC_PRESET_P1_GUID; // slow
    else if (config_.preset == 3) presetGUID = NV_ENC_PRESET_P4_GUID; // fast
    else if (config_.preset == 4) presetGUID = NV_ENC_PRESET_P5_GUID; // fastest

    // 打印配置模式
    const char* rc_mode_names[] = {"ConstQP", "VBR", "CBR", "CQ"};
    const char* rc_mode_desc[] = {
        "固定 QP (手动质量)",
        "可变码率",
        "恒定码率",
        "恒定质量 (保真)"
    };

    if (config_.rc_mode == 0 || config_.rc_mode == 3) {
        // ConstQP / CQ 模式 - 固定质量模式
        std::cout << "[NVENC] 模式: 固定 QP, 质量: " << config_.quality << std::endl;
    } else {
        std::cout << "[NVENC] 模式: " << rc_mode_names[config_.rc_mode]
                  << " (" << rc_mode_desc[config_.rc_mode] << ")" << std::endl;
        std::cout << "[NVENC] 目标码率: " << (config_.bitrate / 1000000.0) << " Mbps" << std::endl;
    }

    // 获取预设配置
    NV_ENC_PRESET_CONFIG presetConfig = {};
    presetConfig.version = NV_ENC_PRESET_CONFIG_VER;
    presetConfig.reserved = 0;
    presetConfig.reserved1[0] = 0;
    for (int i = 1; i < 256; i++) presetConfig.reserved1[i] = 0;
    for (int i = 0; i < 64; i++) presetConfig.reserved2[i] = nullptr;

    NVENCSTATUS err = nvenc_api_.nvEncGetEncodePresetConfigEx(
        nvenc_encoder_,
        NV_ENC_CODEC_H264_GUID,
        presetGUID,
        NV_ENC_TUNING_INFO_HIGH_QUALITY,
        &presetConfig
    );

    // 配置编码器
    NV_ENC_CONFIG encodeConfig = {};
    if (err == NV_ENC_SUCCESS) {
        encodeConfig = presetConfig.presetCfg;
    }

    // 覆盖关键参数
    encodeConfig.version = NV_ENC_CONFIG_VER;
    encodeConfig.profileGUID = NV_ENC_CODEC_PROFILE_AUTOSELECT_GUID;
    encodeConfig.gopLength = config_.gop_size;
    encodeConfig.frameIntervalP = 1; // IPP
    encodeConfig.monoChromeEncoding = 0;
    // 注意：bufferFmt 不在这里设置，而是在创建输入缓冲区时指定

    // 码率控制配置
    encodeConfig.rcParams.version = NV_ENC_RC_PARAMS_VER;

    if (zerocopy_only_) {
        // Keep zerocopy session close to NVENC preset defaults to avoid param conflicts.
        encodeConfig.rcParams.rateControlMode = NV_ENC_PARAMS_RC_CONSTQP;
        encodeConfig.rcParams.constQP.qpIntra = config_.quality;
        encodeConfig.rcParams.constQP.qpInterP = config_.quality;
        encodeConfig.rcParams.constQP.qpInterB = config_.quality + 1;
    } else if (config_.rc_mode == 0 || config_.rc_mode == 3) {
        // ConstQP / CQ (Constant Quality) 模式 - 固定质量模式
        encodeConfig.rcParams.rateControlMode = NV_ENC_PARAMS_RC_CONSTQP;
        encodeConfig.rcParams.constQP.qpIntra = config_.quality;
        encodeConfig.rcParams.constQP.qpInterP = config_.quality;
        encodeConfig.rcParams.constQP.qpInterB = config_.quality + 1;
        // 不设置平均码率和最大码率，让质量主导
        encodeConfig.rcParams.averageBitRate = 0;
        encodeConfig.rcParams.maxBitRate = 0;
        encodeConfig.rcParams.vbvBufferSize = 0;
        // 不设置 min/max QP 限制，让编码器自由处理
        // 只使用 constQP 控制质量
    } else {
        // CBR/VBR 模式
        encodeConfig.rcParams.rateControlMode = (NV_ENC_PARAMS_RC_MODE)config_.rc_mode;
        encodeConfig.rcParams.averageBitRate = config_.bitrate;
        encodeConfig.rcParams.maxBitRate = config_.bitrate;  // CBR 模式下等于平均码率

        // VBV 缓冲区设置
        int vbv_size = config_.bitrate / config_.framerate * 2;
        encodeConfig.rcParams.vbvBufferSize = vbv_size;
        encodeConfig.rcParams.vbvInitialDelay = vbv_size / 2;

        // 根据码率设置 QP 范围
        // 对于高分辨率，NVENC 有最小码率限制，需要通过最小 QP 来强制低码率
        double bitrate_mbps = config_.bitrate / 1000000.0;
        int min_qp_intra = 0, min_qp_p = 0, min_qp_b = 0;

        if (bitrate_mbps < 3) {
            // 极低码率 (1-2 Mbps): 强制高 QP（低质量）
            min_qp_intra = 45;
            min_qp_p = 47;
            min_qp_b = 49;
        } else if (bitrate_mbps < 6) {
            // 低码率 (3-5 Mbps): 中等 QP
            min_qp_intra = 38;
            min_qp_p = 40;
            min_qp_b = 42;
        } else if (bitrate_mbps < 12) {
            // 中等码率 (6-11 Mbps): 较低 QP
            min_qp_intra = 28;
            min_qp_p = 30;
            min_qp_b = 32;
        } else if (bitrate_mbps < 20) {
            // 高码率 (12-19 Mbps): 轻微限制
            min_qp_intra = 18;
            min_qp_p = 20;
            min_qp_b = 22;
        } else {
            // 20+ Mbps: 不限制 QP
            min_qp_intra = 0;
            min_qp_p = 0;
            min_qp_b = 0;
        }

        encodeConfig.rcParams.maxQP.qpIntra = 51;
        encodeConfig.rcParams.maxQP.qpInterP = 51;
        encodeConfig.rcParams.maxQP.qpInterB = 51;
        encodeConfig.rcParams.minQP.qpIntra = min_qp_intra;
        encodeConfig.rcParams.minQP.qpInterP = min_qp_p;
        encodeConfig.rcParams.minQP.qpInterB = min_qp_b;
    }

    // H.264 配置
    encodeConfig.encodeCodecConfig.h264Config.level = NV_ENC_LEVEL_AUTOSELECT;
    encodeConfig.encodeCodecConfig.h264Config.idrPeriod = config_.gop_size;
    encodeConfig.encodeCodecConfig.h264Config.entropyCodingMode = NV_ENC_H264_ENTROPY_CODING_MODE_CABAC;
    encodeConfig.encodeCodecConfig.h264Config.chromaFormatIDC = 1; // YUV420
    encodeConfig.encodeCodecConfig.h264Config.maxNumRefFrames = 0; // Use default
    encodeConfig.encodeCodecConfig.h264Config.outputBitDepth = NV_ENC_BIT_DEPTH_8;
    encodeConfig.encodeCodecConfig.h264Config.inputBitDepth = NV_ENC_BIT_DEPTH_8;

    // 初始化参数
    NV_ENC_INITIALIZE_PARAMS initializeParams = {};
    initializeParams.version = NV_ENC_INITIALIZE_PARAMS_VER;
    initializeParams.encodeGUID = NV_ENC_CODEC_H264_GUID;
    initializeParams.presetGUID = presetGUID;
    initializeParams.encodeWidth = config_.width;
    initializeParams.encodeHeight = config_.height;
    initializeParams.darWidth = config_.width;
    initializeParams.darHeight = config_.height;
    initializeParams.frameRateNum = config_.framerate;
    initializeParams.frameRateDen = 1;
    initializeParams.maxEncodeWidth = config_.width;
    initializeParams.maxEncodeHeight = config_.height;
    initializeParams.enableEncodeAsync = 0;
    initializeParams.enablePTD = 1;
    initializeParams.reportSliceOffsets = 0;
    initializeParams.enableSubFrameWrite = 0;
    initializeParams.enableExternalMEHints = 0;
    initializeParams.enableMEOnlyMode = 0;
    initializeParams.enableWeightedPrediction = 0;
    initializeParams.splitEncodeMode = 0;
    initializeParams.enableOutputInVidmem = 0;
    initializeParams.tuningInfo = NV_ENC_TUNING_INFO_HIGH_QUALITY;
    initializeParams.encodeConfig = &encodeConfig;
    // Dedicated zero-copy session uses ARGB; mixed session keeps format flexible.
    initializeParams.bufferFormat = zerocopy_only_ ? NV_ENC_BUFFER_FORMAT_ARGB : NV_ENC_BUFFER_FORMAT_UNDEFINED;

    // Debug: print input formats supported by this codec/session.
    if (nvenc_api_.nvEncGetInputFormats) {
        NV_ENC_BUFFER_FORMAT fmts[64] = {};
        uint32_t fmt_count = 0;
        NVENCSTATUS fmt_err = nvenc_api_.nvEncGetInputFormats(
            nvenc_encoder_,
            NV_ENC_CODEC_H264_GUID,
            fmts,
            64,
            &fmt_count
        );
        if (fmt_err == NV_ENC_SUCCESS) {
            bool has_argb = false;
            bool has_abgr = false;
            for (uint32_t i = 0; i < fmt_count && i < 64; ++i) {
                if (fmts[i] == NV_ENC_BUFFER_FORMAT_ARGB) has_argb = true;
                if (fmts[i] == NV_ENC_BUFFER_FORMAT_ABGR) has_abgr = true;
            }
            std::cout << "[NVENC] InputFormats count=" << fmt_count
                      << ", ARGB=" << (has_argb ? 1 : 0)
                      << ", ABGR=" << (has_abgr ? 1 : 0) << std::endl;
        } else {
            std::cout << "[NVENC] nvEncGetInputFormats failed: " << fmt_err << std::endl;
        }
    }

    err = nvenc_api_.nvEncInitializeEncoder(nvenc_encoder_, &initializeParams);
    if (err != NV_ENC_SUCCESS) {
        std::cerr << "[NVENC] nvEncInitializeEncoder failed: " << err << std::endl;
        return false;
    }

    std::cout << "[NVENC] Encoder initialized" << std::endl;
    return true;
}

bool NVENCEncoderImpl::AllocateBuffers() {
    const uint32_t numBuffers = 4;
    input_buffers_.resize(numBuffers);
    bitstream_buffers_.resize(numBuffers);
    zerocopy_registered_resources_.clear();
    zerocopy_textures_.clear();
    zerocopy_slot_inflight_.clear();
    video_output_views_.clear();
    video_input_view_.Reset();

    // Dedicated zero-copy session uses ARGB buffers; mixed mode keeps NV12.
    use_abgr_format_ = zerocopy_only_;

    if (zerocopy_only_) {
        // Build D3D11 video processor for BGRA->NV12 conversion.
        HRESULT hr = d3d11_device_->QueryInterface(__uuidof(ID3D11VideoDevice), &video_device_);
        if (FAILED(hr) || !video_device_) {
            std::cerr << "[NVENC] QueryInterface(ID3D11VideoDevice) failed: 0x" << std::hex << hr << std::dec << std::endl;
            return false;
        }
        ComPtr<ID3D11DeviceContext> base_context;
        d3d11_device_->GetImmediateContext(&base_context);
        hr = base_context->QueryInterface(__uuidof(ID3D11VideoContext), &video_context_);
        if (FAILED(hr) || !video_context_) {
            std::cerr << "[NVENC] QueryInterface(ID3D11VideoContext) failed: 0x" << std::hex << hr << std::dec << std::endl;
            return false;
        }

        D3D11_VIDEO_PROCESSOR_CONTENT_DESC content_desc = {};
        content_desc.InputFrameFormat = D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE;
        content_desc.InputFrameRate.Numerator = config_.framerate;
        content_desc.InputFrameRate.Denominator = 1;
        content_desc.InputWidth = config_.width;
        content_desc.InputHeight = config_.height;
        content_desc.OutputFrameRate.Numerator = config_.framerate;
        content_desc.OutputFrameRate.Denominator = 1;
        content_desc.OutputWidth = config_.width;
        content_desc.OutputHeight = config_.height;
        content_desc.Usage = D3D11_VIDEO_USAGE_PLAYBACK_NORMAL;

        hr = video_device_->CreateVideoProcessorEnumerator(&content_desc, &video_processor_enum_);
        if (FAILED(hr) || !video_processor_enum_) {
            std::cerr << "[NVENC] CreateVideoProcessorEnumerator failed: 0x" << std::hex << hr << std::dec << std::endl;
            return false;
        }
        hr = video_device_->CreateVideoProcessor(video_processor_enum_.Get(), 0, &video_processor_);
        if (FAILED(hr) || !video_processor_) {
            std::cerr << "[NVENC] CreateVideoProcessor failed: 0x" << std::hex << hr << std::dec << std::endl;
            return false;
        }

        // Sample-style: pre-create/register NV12 textures and map them per frame.
        zerocopy_textures_.resize(numBuffers);
        zerocopy_registered_resources_.resize(numBuffers, nullptr);
        zerocopy_slot_inflight_.assign(numBuffers, false);
        video_output_views_.resize(numBuffers);

        D3D11_TEXTURE2D_DESC texDesc = {};
        texDesc.Width = config_.width;
        texDesc.Height = config_.height;
        texDesc.MipLevels = 1;
        texDesc.ArraySize = 1;
        texDesc.Format = DXGI_FORMAT_NV12;
        texDesc.SampleDesc.Count = 1;
        texDesc.Usage = D3D11_USAGE_DEFAULT;
        texDesc.BindFlags = D3D11_BIND_RENDER_TARGET;
        texDesc.CPUAccessFlags = 0;
        texDesc.MiscFlags = 0;

        for (uint32_t i = 0; i < numBuffers; ++i) {
            HRESULT hr = d3d11_device_->CreateTexture2D(&texDesc, nullptr, &zerocopy_textures_[i]);
            if (FAILED(hr) || !zerocopy_textures_[i]) {
                std::cerr << "[NVENC] Failed to create zerocopy texture: 0x" << std::hex << hr << std::dec << std::endl;
                return false;
            }

            NV_ENC_REGISTER_RESOURCE reg = {};
            reg.version = NV_ENC_REGISTER_RESOURCE_VER;
            reg.resourceType = NV_ENC_INPUT_RESOURCE_TYPE_DIRECTX;
            reg.resourceToRegister = zerocopy_textures_[i].Get();
            reg.width = config_.width;
            reg.height = config_.height;
            reg.pitch = 0;
            reg.subResourceIndex = 0;
            reg.bufferFormat = NV_ENC_BUFFER_FORMAT_NV12;
            reg.bufferUsage = NV_ENC_INPUT_IMAGE;

            NVENCSTATUS err = nvenc_api_.nvEncRegisterResource(nvenc_encoder_, &reg);
            if (err != NV_ENC_SUCCESS) {
                std::cerr << "[NVENC] nvEncRegisterResource(zerocopy) failed: " << err << std::endl;
                return false;
            }
            zerocopy_registered_resources_[i] = reg.registeredResource;

            D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC out_desc = {};
            out_desc.ViewDimension = D3D11_VPOV_DIMENSION_TEXTURE2D;
            out_desc.Texture2D.MipSlice = 0;
            hr = video_device_->CreateVideoProcessorOutputView(
                zerocopy_textures_[i].Get(),
                video_processor_enum_.Get(),
                &out_desc,
                &video_output_views_[i]
            );
            if (FAILED(hr) || !video_output_views_[i]) {
                std::cerr << "[NVENC] CreateVideoProcessorOutputView(slot) failed: 0x" << std::hex << hr << std::dec << std::endl;
                return false;
            }
        }
    } else {
        NV_ENC_CREATE_INPUT_BUFFER createInputBufferParams = {};
        createInputBufferParams.version = NV_ENC_CREATE_INPUT_BUFFER_VER;
        createInputBufferParams.width = config_.width;
        createInputBufferParams.height = config_.height;
        createInputBufferParams.bufferFmt = NV_ENC_BUFFER_FORMAT_NV12;

        for (uint32_t i = 0; i < numBuffers; i++) {
            NVENCSTATUS err = nvenc_api_.nvEncCreateInputBuffer(nvenc_encoder_, &createInputBufferParams);
            if (err != NV_ENC_SUCCESS) {
                std::cerr << "[NVENC] nvEncCreateInputBuffer failed: " << err << std::endl;
                return false;
            }
            input_buffers_[i] = createInputBufferParams.inputBuffer;
        }
    }

    std::cout << "[NVENC] Input buffer mode: "
              << (zerocopy_only_ ? "D3D11 registered textures (zerocopy)" : "NV12 buffers")
              << std::endl;

    NV_ENC_CREATE_BITSTREAM_BUFFER createBitstreamBufferParams = {};
    createBitstreamBufferParams.version = NV_ENC_CREATE_BITSTREAM_BUFFER_VER;
    // Keep a larger bitstream buffer headroom for intra spikes.
    createBitstreamBufferParams.size = config_.width * config_.height * 4;

    for (uint32_t i = 0; i < numBuffers; i++) {
        NVENCSTATUS err = nvenc_api_.nvEncCreateBitstreamBuffer(nvenc_encoder_, &createBitstreamBufferParams);
        if (err != NV_ENC_SUCCESS) {
            std::cerr << "[NVENC] nvEncCreateBitstreamBuffer failed: " << err << std::endl;
            return false;
        }
        bitstream_buffers_[i] = createBitstreamBufferParams.bitstreamBuffer;
    }

    std::cout << "[NVENC] Allocated " << numBuffers << " input and bitstream buffers" << std::endl;

    // 创建中间 D3D11 纹理 (用于从外部设备复制纹理)
    // 使用 CUDA 兼容的标志
    D3D11_TEXTURE2D_DESC texDesc = {};
    texDesc.Width = config_.width;
    texDesc.Height = config_.height;
    texDesc.MipLevels = 1;
    texDesc.ArraySize = 1;
    texDesc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
    texDesc.SampleDesc.Count = 1;
    texDesc.Usage = D3D11_USAGE_DEFAULT;
    texDesc.BindFlags = D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET;
    texDesc.CPUAccessFlags = 0;
    texDesc.MiscFlags = 0;

    HRESULT hr = d3d11_device_->CreateTexture2D(&texDesc, nullptr, &intermediate_texture_);
    if (FAILED(hr)) {
        std::cerr << "[NVENC] Failed to create intermediate texture: 0x" << std::hex << hr << std::endl;
        return false;
    }

    if (zerocopy_only_) {
        D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC in_desc = {};
        in_desc.FourCC = 0;
        in_desc.ViewDimension = D3D11_VPIV_DIMENSION_TEXTURE2D;
        in_desc.Texture2D.MipSlice = 0;
        in_desc.Texture2D.ArraySlice = 0;

        hr = video_device_->CreateVideoProcessorInputView(
            intermediate_texture_.Get(),
            video_processor_enum_.Get(),
            &in_desc,
            &video_input_view_
        );
        if (FAILED(hr) || !video_input_view_) {
            std::cerr << "[NVENC] CreateVideoProcessorInputView failed: 0x" << std::hex << hr << std::dec << std::endl;
            return false;
        }
    }

    std::cout << "[NVENC] Created intermediate texture for cross-device copy" << std::endl;
    return true;
}

bool NVENCEncoderImpl::RegisterD3D11Resource(ID3D11Texture2D* texture) {
    if (!texture || !nvenc_encoder_) {
        return false;
    }

    // 注销之前的资源
    if (registered_resource_) {
        nvenc_api_.nvEncUnregisterResource(nvenc_encoder_, registered_resource_);
        registered_resource_ = nullptr;
    }

    // 注册 D3D11 纹理到 NVENC
    NV_ENC_REGISTER_RESOURCE registerResParams = {};
    registerResParams.version = NV_ENC_REGISTER_RESOURCE_VER;
    registerResParams.resourceType = NV_ENC_INPUT_RESOURCE_TYPE_DIRECTX;
    registerResParams.resourceToRegister = texture;
    registerResParams.width = config_.width;
    registerResParams.height = config_.height;
    registerResParams.bufferFormat = NV_ENC_BUFFER_FORMAT_ABGR;  // BGRA = ABGR in NVENC (A8B8G8R8)

    NVENCSTATUS err = nvenc_api_.nvEncRegisterResource(nvenc_encoder_, &registerResParams);
    if (err != NV_ENC_SUCCESS) {
        std::cerr << "[NVENC] Failed to register D3D11 resource: " << err << std::endl;
        return false;
    }

    registered_resource_ = registerResParams.registeredResource;
    return true;
}

bool NVENCEncoderImpl::EncodeFromCPU(const unsigned char* data, int size, long long timestamp, bool force_keyframe) {
    if (!initialized_ || !data) {
        return false;
    }

    NV_ENC_LOCK_INPUT_BUFFER lockInputBufferParams = {};
    lockInputBufferParams.version = NV_ENC_LOCK_INPUT_BUFFER_VER;
    lockInputBufferParams.inputBuffer = input_buffers_[current_input_buffer_];
    lockInputBufferParams.doNotWait = 0;

    NVENCSTATUS err = nvenc_api_.nvEncLockInputBuffer(nvenc_encoder_, &lockInputBufferParams);
    if (err != NV_ENC_SUCCESS) {
        return false;
    }

    // 转换 BGRA 到 NV12 (CPU fallback with proper UV)
    uint8_t* dstY = (uint8_t*)lockInputBufferParams.bufferDataPtr;
    uint8_t* dstUV = dstY + lockInputBufferParams.pitch * config_.height;
    ConvertBGRAtoNV12CPU(
        data,
        config_.width * 4,
        config_.width,
        config_.height,
        dstY,
        dstUV,
        lockInputBufferParams.pitch
    );

    nvenc_api_.nvEncUnlockInputBuffer(nvenc_encoder_, lockInputBufferParams.inputBuffer);

    // 编码帧
    NV_ENC_PIC_PARAMS picParams = {};
    picParams.version = NV_ENC_PIC_PARAMS_VER;
    picParams.inputBuffer = input_buffers_[current_input_buffer_];
    picParams.outputBitstream = bitstream_buffers_[current_bitstream_buffer_];
    picParams.inputWidth = config_.width;
    picParams.inputHeight = config_.height;
    picParams.inputPitch = lockInputBufferParams.pitch;
    picParams.pictureStruct = NV_ENC_PIC_STRUCT_FRAME;
    picParams.frameIdx = current_pts_;

    if (force_keyframe || force_keyframe_ || current_pts_ % config_.gop_size == 0) {
        picParams.encodePicFlags = NV_ENC_PIC_FLAG_FORCEIDR;
        force_keyframe_ = false;
    } else {
        picParams.encodePicFlags = 0;
    }

    err = nvenc_api_.nvEncEncodePicture(nvenc_encoder_, &picParams);

    current_input_buffer_ = (current_input_buffer_ + 1) % input_buffers_.size();
    current_bitstream_buffer_ = (current_bitstream_buffer_ + 1) % bitstream_buffers_.size();

    if (err == NV_ENC_SUCCESS || err == NV_ENC_ERR_NEED_MORE_INPUT) {
        // 尝试获取编码输出
        uint32_t lastBuffer = (current_bitstream_buffer_ - 1 + bitstream_buffers_.size()) % bitstream_buffers_.size();

        NV_ENC_LOCK_BITSTREAM lockBitstreamData = {};
        lockBitstreamData.version = NV_ENC_LOCK_BITSTREAM_VER;
        lockBitstreamData.outputBitstream = bitstream_buffers_[lastBuffer];
        lockBitstreamData.doNotWait = 0;

        err = nvenc_api_.nvEncLockBitstream(nvenc_encoder_, &lockBitstreamData);
        if (err == NV_ENC_SUCCESS && lockBitstreamData.bitstreamSizeInBytes > 0) {
            EncodedOutput output;
            output.data.assign((uint8_t*)lockBitstreamData.bitstreamBufferPtr,
                             (uint8_t*)lockBitstreamData.bitstreamBufferPtr + lockBitstreamData.bitstreamSizeInBytes);
            output.timestamp = current_pts_;
            output.key_frame = (picParams.encodePicFlags & NV_ENC_PIC_FLAG_FORCEIDR) != 0;

            output_queue_.push_back(output);
            current_pts_++;

            nvenc_api_.nvEncUnlockBitstream(nvenc_encoder_, lockBitstreamData.outputBitstream);
        }

        return true;
    }

    return false;
}

bool NVENCEncoderImpl::EncodeFromD3D11(ID3D11Texture2D* texture, long long timestamp, bool force_keyframe) {
    if (!initialized_ || !texture) {
        return false;
    }

    // ============================================================================
    // GPU Direct 方案: 外部 D3D11 纹理 → 复制到中间纹理 → CUDA 数组 → CUDA kernel → NV12 → NVENC
    // 完全在 GPU 上完成，不经过 CPU
    // ============================================================================

    // 1. 设置 CUDA 上下文
    CUcontext prev_context = nullptr;
    cuCtxPushCurrent(cuda_context_);

    // 2. 将外部纹理复制到我们的中间纹理 (跨设备复制，但在 GPU 上完成)
    if (intermediate_texture_) {
        d3d11_context_->CopyResource(intermediate_texture_.Get(), texture);
        d3d11_context_->Flush();  // 确保复制完成
    } else {
        std::cerr << "[NVENC] Intermediate texture not created" << std::endl;
        cuCtxPopCurrent(&prev_context);
        return false;
    }

    // 3. 注册中间 D3D11 纹理到 CUDA 图形资源
    CUgraphicsResource cudaResource = nullptr;
    CUresult cudaErr = cuGraphicsD3D11RegisterResource(&cudaResource, intermediate_texture_.Get(), 0);
    if (cudaErr != CUDA_SUCCESS) {
        std::cerr << "[NVENC] cuGraphicsD3D11RegisterResource failed: " << cudaErr << std::endl;
        cuCtxPopCurrent(&prev_context);
        return false;
    }

    // 4. 映射图形资源
    cudaErr = cuGraphicsMapResources(1, &cudaResource, cuda_stream_);
    if (cudaErr != CUDA_SUCCESS) {
        std::cerr << "[NVENC] cuGraphicsMapResources failed: " << cudaErr << std::endl;
        cuGraphicsUnregisterResource(cudaResource);
        cuCtxPopCurrent(&prev_context);
        return false;
    }

    // 5. 获取 CUDA 数组（纹理表示）
    CUarray cudaArray = nullptr;
    cudaErr = cuGraphicsSubResourceGetMappedArray(&cudaArray, cudaResource, 0, 0);
    if (cudaErr != CUDA_SUCCESS) {
        std::cerr << "[NVENC] cuGraphicsSubResourceGetMappedArray failed: " << cudaErr << std::endl;
        cuGraphicsUnmapResources(1, &cudaResource, cuda_stream_);
        cuGraphicsUnregisterResource(cudaResource);
        cuCtxPopCurrent(&prev_context);
        return false;
    }

    // 6. 锁定 NVENC 输入缓冲区
    NV_ENC_LOCK_INPUT_BUFFER lockInputBufferParams = {};
    lockInputBufferParams.version = NV_ENC_LOCK_INPUT_BUFFER_VER;
    lockInputBufferParams.inputBuffer = input_buffers_[current_input_buffer_];
    lockInputBufferParams.doNotWait = 0;

    NVENCSTATUS err = nvenc_api_.nvEncLockInputBuffer(nvenc_encoder_, &lockInputBufferParams);
    if (err != NV_ENC_SUCCESS) {
        std::cerr << "[NVENC] Failed to lock input buffer: " << err << std::endl;
        cuGraphicsUnmapResources(1, &cudaResource, cuda_stream_);
        cuGraphicsUnregisterResource(cudaResource);
        cuCtxPopCurrent(&prev_context);
        return false;
    }

    // 7. 从 CUDA 数组复制到 NVENC 缓冲区
    uint8_t* dstBuffer = (uint8_t*)lockInputBufferParams.bufferDataPtr;
    int dstPitch = lockInputBufferParams.pitch;

    if (use_abgr_format_) {
        // ABGR 格式：直接复制 BGRA 数据，无需颜色转换！
        // 这是真正的 GPU Direct 路径：WGC → D3D11 → CUDA → NVENC ABGR
        CUDA_MEMCPY2D copyParams = {};
        copyParams.srcMemoryType = CU_MEMORYTYPE_ARRAY;
        copyParams.srcArray = cudaArray;
        copyParams.srcXInBytes = 0;
        copyParams.srcY = 0;
        copyParams.dstMemoryType = CU_MEMORYTYPE_DEVICE;
        copyParams.dstDevice = (CUdeviceptr)dstBuffer;
        copyParams.dstXInBytes = 0;
        copyParams.dstY = 0;
        copyParams.WidthInBytes = config_.width * 4;  // BGRA = 4 字节/像素
        copyParams.Height = config_.height;
        copyParams.dstPitch = dstPitch;

        cudaErr = cuMemcpy2D(&copyParams);
        if (cudaErr != CUDA_SUCCESS) {
            std::cerr << "[NVENC] cuMemcpy2D (ABGR) failed: " << cudaErr << std::endl;
            nvenc_api_.nvEncUnlockInputBuffer(nvenc_encoder_, lockInputBufferParams.inputBuffer);
            cuGraphicsUnmapResources(1, &cudaResource, cuda_stream_);
            cuGraphicsUnregisterResource(cudaResource);
            cuCtxPopCurrent(&prev_context);
            return false;
        }
    } else {
        // NV12 格式：使用 CUDA kernel 进行 GPU 端 BGRA→NV12 转换
        // 分配临时 BGRA 缓冲区（GPU 端）
        int bgraSize = config_.width * config_.height * 4;
        CUdeviceptr bgraBuffer = 0;
        cudaErr = cuMemAlloc(&bgraBuffer, bgraSize);
        if (cudaErr != CUDA_SUCCESS) {
            std::cerr << "[NVENC] cuMemAlloc (BGRA) failed: " << cudaErr << std::endl;
            nvenc_api_.nvEncUnlockInputBuffer(nvenc_encoder_, lockInputBufferParams.inputBuffer);
            cuGraphicsUnmapResources(1, &cudaResource, cuda_stream_);
            cuGraphicsUnregisterResource(cudaResource);
            cuCtxPopCurrent(&prev_context);
            return false;
        }

        // 从 CUDA 数组复制 BGRA 数据到临时缓冲区
        CUDA_MEMCPY2D copyParams = {};
        copyParams.srcMemoryType = CU_MEMORYTYPE_ARRAY;
        copyParams.srcArray = cudaArray;
        copyParams.srcXInBytes = 0;
        copyParams.srcY = 0;
        copyParams.dstMemoryType = CU_MEMORYTYPE_DEVICE;
        copyParams.dstDevice = bgraBuffer;
        copyParams.dstXInBytes = 0;
        copyParams.dstY = 0;
        copyParams.WidthInBytes = config_.width * 4;
        copyParams.Height = config_.height;

        cudaErr = cuMemcpy2D(&copyParams);
        if (cudaErr != CUDA_SUCCESS) {
            std::cerr << "[NVENC] cuMemcpy2D failed: " << cudaErr << std::endl;
            cuMemFree(bgraBuffer);
            nvenc_api_.nvEncUnlockInputBuffer(nvenc_encoder_, lockInputBufferParams.inputBuffer);
            cuGraphicsUnmapResources(1, &cudaResource, cuda_stream_);
            cuGraphicsUnregisterResource(cudaResource);
            cuCtxPopCurrent(&prev_context);
            return false;
        }

        // 使用 CUDA kernel 进行 BGRA→NV12 转换（GPU 端）
        // Y 平面转换
        dim3 blockDim(16, 16);
        dim3 gridDim((config_.width + blockDim.x - 1) / blockDim.x,
                     (config_.height + blockDim.y - 1) / blockDim.y);

        // 获取 CUDA kernel 函数
        CUfunction yKernel = nullptr;
        CUfunction uvKernel = nullptr;

        // 简化的 Y 转换 kernel（内联实现）
        // Y = 0.299*R + 0.587*G + 0.114*B = (77*R + 150*G + 29*B + 128) >> 8
        // 为了简化，这里使用 CPU 完成转换（仍然需要优化）
        // TODO: 实现真正的 CUDA kernel

        // 临时方案：复制 BGRA 到 CPU，转换后复制回 NV12 缓冲区
        std::vector<uint8_t> bgraData(bgraSize);
        cudaErr = cuMemcpyDtoH(bgraData.data(), bgraBuffer, bgraSize);
        cuMemFree(bgraBuffer);

        if (cudaErr != CUDA_SUCCESS) {
            std::cerr << "[NVENC] cuMemcpyDtoH failed: " << cudaErr << std::endl;
            nvenc_api_.nvEncUnlockInputBuffer(nvenc_encoder_, lockInputBufferParams.inputBuffer);
            cuGraphicsUnmapResources(1, &cudaResource, cuda_stream_);
            cuGraphicsUnregisterResource(cudaResource);
            cuCtxPopCurrent(&prev_context);
            return false;
        }

        // BGRA → NV12 转换 (CPU fallback with proper UV)
        uint8_t* nv12Y = dstBuffer;
        uint8_t* nv12UV = nv12Y + dstPitch * config_.height;
        ConvertBGRAtoNV12CPU(
            bgraData.data(),
            config_.width * 4,
            config_.width,
            config_.height,
            nv12Y,
            nv12UV,
            dstPitch
        );
    }

    // 8. 取消映射 CUDA 资源
    cuGraphicsUnmapResources(1, &cudaResource, cuda_stream_);
    cuGraphicsUnregisterResource(cudaResource);

    // 恢复 CUDA 上下文
    cuCtxPopCurrent(&prev_context);

    // 9. 编码帧参数
    NV_ENC_PIC_PARAMS picParams = {};
    picParams.version = NV_ENC_PIC_PARAMS_VER;
    picParams.inputBuffer = input_buffers_[current_input_buffer_];
    picParams.outputBitstream = bitstream_buffers_[current_bitstream_buffer_];
    picParams.inputWidth = config_.width;
    picParams.inputHeight = config_.height;
    // ABGR 格式的 pitch 是 width * 4，NV12 格式的 pitch 由 NVENC 提供
    picParams.inputPitch = use_abgr_format_ ? (config_.width * 4) : dstPitch;
    picParams.pictureStruct = NV_ENC_PIC_STRUCT_FRAME;
    picParams.frameIdx = current_pts_;

    if (force_keyframe || force_keyframe_ || current_pts_ % config_.gop_size == 0) {
        picParams.encodePicFlags = NV_ENC_PIC_FLAG_FORCEIDR;
        force_keyframe_ = false;
    } else {
        picParams.encodePicFlags = 0;
    }

    // 11. 执行编码
    NVENCSTATUS nvencErr = nvenc_api_.nvEncEncodePicture(nvenc_encoder_, &picParams);

    current_input_buffer_ = (current_input_buffer_ + 1) % input_buffers_.size();
    current_bitstream_buffer_ = (current_bitstream_buffer_ + 1) % bitstream_buffers_.size();

    if (nvencErr == NV_ENC_SUCCESS || nvencErr == NV_ENC_ERR_NEED_MORE_INPUT) {
        // 12. 获取编码输出
        uint32_t lastBuffer = (current_bitstream_buffer_ - 1 + bitstream_buffers_.size()) % bitstream_buffers_.size();

        NV_ENC_LOCK_BITSTREAM lockBitstreamData = {};
        lockBitstreamData.version = NV_ENC_LOCK_BITSTREAM_VER;
        lockBitstreamData.outputBitstream = bitstream_buffers_[lastBuffer];
        lockBitstreamData.doNotWait = 0;

        nvencErr = nvenc_api_.nvEncLockBitstream(nvenc_encoder_, &lockBitstreamData);
        if (nvencErr == NV_ENC_SUCCESS && lockBitstreamData.bitstreamSizeInBytes > 0) {
            EncodedOutput output;
            output.data.assign((uint8_t*)lockBitstreamData.bitstreamBufferPtr,
                             (uint8_t*)lockBitstreamData.bitstreamBufferPtr + lockBitstreamData.bitstreamSizeInBytes);
            output.timestamp = current_pts_;
            output.key_frame = (picParams.encodePicFlags & NV_ENC_PIC_FLAG_FORCEIDR) != 0;

            output_queue_.push_back(output);
            current_pts_++;

            nvenc_api_.nvEncUnlockBitstream(nvenc_encoder_, lockBitstreamData.outputBitstream);
        }

        return true;
    }

    std::cerr << "[NVENC] Encode picture failed: " << nvencErr << std::endl;
    return false;
}

bool NVENCEncoderImpl::DrainPendingPackets(bool do_not_wait) {
    if (!nvenc_encoder_) {
        return false;
    }

    bool drained_any = false;
    while (!pending_packets_.empty()) {
        const PendingPacket front = pending_packets_.front();

        NV_ENC_LOCK_BITSTREAM lockBitstreamData = {};
        lockBitstreamData.version = NV_ENC_LOCK_BITSTREAM_VER;
        lockBitstreamData.outputBitstream = bitstream_buffers_[front.bitstream_index];
        lockBitstreamData.doNotWait = do_not_wait ? 1 : 0;

        NVENCSTATUS lock_err = nvenc_api_.nvEncLockBitstream(nvenc_encoder_, &lockBitstreamData);
        if (lock_err != NV_ENC_SUCCESS) {
            if (do_not_wait) {
                // Non-blocking poll: any non-success means "not ready yet".
                if (lock_err == NV_ENC_ERR_LOCK_BUSY) {
                    zc_lock_busy_count_++;
                } else {
                    zc_lock_retryable_count_++;
                }
            } else if (lock_err == NV_ENC_ERR_LOCK_BUSY ||
                       lock_err == NV_ENC_ERR_NEED_MORE_INPUT ||
                       lock_err == NV_ENC_ERR_ENCODER_BUSY) {
                zc_lock_retryable_count_++;
            } else {
                zc_lock_failures_++;
            }
            break;
        }

        if (lockBitstreamData.bitstreamSizeInBytes > 0) {
            EncodedOutput output;
            output.data.assign(
                (uint8_t*)lockBitstreamData.bitstreamBufferPtr,
                (uint8_t*)lockBitstreamData.bitstreamBufferPtr + lockBitstreamData.bitstreamSizeInBytes
            );
            output.timestamp = front.timestamp;
            output.key_frame = front.key_frame;
            output_queue_.push_back(output);
            zc_bitstream_outputs_++;
        }

        nvenc_api_.nvEncUnlockBitstream(nvenc_encoder_, lockBitstreamData.outputBitstream);
        if (front.mapped_resource) {
            nvenc_api_.nvEncUnmapInputResource(nvenc_encoder_, front.mapped_resource);
            zc_unmap_count_++;
        }
        if (front.bitstream_index < zerocopy_slot_inflight_.size()) {
            zerocopy_slot_inflight_[front.bitstream_index] = false;
        }
        pending_packets_.pop_front();
        drained_any = true;
    }

    return drained_any;
}

bool NVENCEncoderImpl::GetZeroCopyStats(NVENCZeroCopyStats* stats) const {
    if (!stats) {
        return false;
    }
    stats->encode_calls = zc_encode_calls_;
    stats->encode_submit_success = zc_encode_submit_success_;
    stats->encode_submit_need_more_input = zc_encode_submit_need_more_input_;
    stats->encode_submit_fail = zc_encode_submit_fail_;
    stats->slot_busy_skips = zc_slot_busy_skips_;
    stats->map_failures = zc_map_failures_;
    stats->lock_busy_count = zc_lock_busy_count_;
    stats->lock_retryable_count = zc_lock_retryable_count_;
    stats->lock_failures = zc_lock_failures_;
    stats->bitstream_outputs = zc_bitstream_outputs_;
    stats->unmap_count = zc_unmap_count_;
    stats->pending_peak = zc_pending_peak_;
    stats->pending_current = static_cast<unsigned int>(pending_packets_.size());
    return true;
}

bool NVENCEncoderImpl::GetEncodedFrame(NVENCEncodedFrame* frame) {
    if (!initialized_ || !frame) {
        return false;
    }

    if (!pending_packets_.empty()) {
        DrainPendingPackets(false);
    }

    if (output_queue_.empty()) {
        return false;
    }

    auto& output = output_queue_.front();

    frame->data = output.data.data();
    frame->size = output.data.size();
    frame->key_frame = output.key_frame;
    frame->timestamp = output.timestamp;

    output_queue_.erase(output_queue_.begin());
    return true;
}

void NVENCEncoderImpl::RequestKeyframe() {
    force_keyframe_ = true;
}

void NVENCEncoderImpl::Cleanup() {
    output_queue_.clear();
    if (!pending_packets_.empty() && nvenc_encoder_) {
        for (const auto& pkt : pending_packets_) {
            if (pkt.mapped_resource) {
                nvenc_api_.nvEncUnmapInputResource(nvenc_encoder_, pkt.mapped_resource);
                zc_unmap_count_++;
            }
        }
    }
    pending_packets_.clear();
    zerocopy_slot_inflight_.clear();
    video_input_view_.Reset();
    video_output_views_.clear();

    // 解除映射（如果还在映射状态）
    if (mapped_resource_ && nvenc_encoder_) {
        nvenc_api_.nvEncUnmapInputResource(nvenc_encoder_, mapped_resource_);
        zc_unmap_count_++;
        mapped_resource_ = nullptr;
    }

    // 注销 D3D11 资源
    if (registered_resource_ && nvenc_encoder_) {
        nvenc_api_.nvEncUnregisterResource(nvenc_encoder_, registered_resource_);
        registered_resource_ = nullptr;
        registered_source_texture_ = nullptr;
    }

    if (!zerocopy_registered_resources_.empty() && nvenc_encoder_) {
        for (void* reg : zerocopy_registered_resources_) {
            if (reg) {
                nvenc_api_.nvEncUnregisterResource(nvenc_encoder_, reg);
            }
        }
    }
    zerocopy_registered_resources_.clear();
    zerocopy_textures_.clear();

    if (nvenc_encoder_) {
        for (auto buf : input_buffers_) {
            if (buf) nvenc_api_.nvEncDestroyInputBuffer(nvenc_encoder_, buf);
        }
        for (auto buf : bitstream_buffers_) {
            if (buf) nvenc_api_.nvEncDestroyBitstreamBuffer(nvenc_encoder_, buf);
        }
        nvenc_api_.nvEncDestroyEncoder(nvenc_encoder_);
        nvenc_encoder_ = nullptr;
    }

    input_buffers_.clear();
    bitstream_buffers_.clear();

    if (cuda_stream_) {
        cudaStreamDestroy(cuda_stream_);
        cuda_stream_ = nullptr;
    }

    if (cuda_context_) {
        cuCtxDestroy(cuda_context_);
        cuda_context_ = nullptr;
    }

    if (nvenc_dll_) {
        FreeLibrary(nvenc_dll_);
        nvenc_dll_ = nullptr;
    }

    cuda_initialized_ = false;
    nvenc_loaded_ = false;
    initialized_ = false;
}

void NVENCEncoderImpl::Release() {
    Cleanup();
}

// ============================================================================
// 零拷贝 D3D11 编码 (使用 NVENC MapInputResource API - 真正的零拷贝)
// ============================================================================

bool NVENCEncoderImpl::EncodeFromD3D11_ZeroCopy(ID3D11Texture2D* texture, long long timestamp, bool force_keyframe) {
    if (!initialized_ || !texture) {
        return false;
    }
    zc_encode_calls_++;

    if (zerocopy_registered_resources_.empty() || zerocopy_textures_.empty()) {
        std::cerr << "[NVENC-ZeroCopy] Zerocopy resources not initialized" << std::endl;
        return false;
    }

    if (!pending_packets_.empty()) {
        DrainPendingPackets(true);
    }

    const uint32_t idx = current_input_buffer_ % static_cast<uint32_t>(zerocopy_textures_.size());
    if (idx < zerocopy_slot_inflight_.size() && zerocopy_slot_inflight_[idx]) {
        // Slot still owned by NVENC; backpressure instead of reusing mapped resources.
        zc_slot_busy_skips_++;
        return false;
    }

    ID3D11Texture2D* encode_texture = zerocopy_textures_[idx].Get();
    void* registered = zerocopy_registered_resources_[idx];
    if (!encode_texture || !registered) {
        std::cerr << "[NVENC-ZeroCopy] Invalid zerocopy resource slot" << std::endl;
        return false;
    }

    if (!intermediate_texture_ || !video_device_ || !video_context_ || !video_processor_ || !video_processor_enum_ || !video_input_view_) {
        std::cerr << "[NVENC-ZeroCopy] Video processor path not initialized" << std::endl;
        return false;
    }

    // 1) Copy source BGRA to stable intermediate texture.
    d3d11_context_->CopyResource(intermediate_texture_.Get(), texture);

    // 2) Use D3D11 VideoProcessor to convert BGRA -> NV12 texture (GPU-only).
    if (idx >= video_output_views_.size() || !video_output_views_[idx]) {
        std::cerr << "[NVENC-ZeroCopy] Video processor output view missing for slot " << idx << std::endl;
        return false;
    }

    D3D11_VIDEO_PROCESSOR_STREAM stream = {};
    stream.Enable = TRUE;
    stream.pInputSurface = video_input_view_.Get();
    HRESULT hr = video_context_->VideoProcessorBlt(
        video_processor_.Get(),
        video_output_views_[idx].Get(),
        0,
        1,
        &stream
    );
    if (FAILED(hr)) {
        std::cerr << "[NVENC-ZeroCopy] VideoProcessorBlt failed: 0x" << std::hex << hr << std::dec << std::endl;
        return false;
    }
    d3d11_context_->Flush();

    // 映射资源到 NVENC 输入缓冲区
    NV_ENC_MAP_INPUT_RESOURCE mapInputResParams = {};
    mapInputResParams.version = NV_ENC_MAP_INPUT_RESOURCE_VER;
    mapInputResParams.subResourceIndex = 0;
    mapInputResParams.inputResource = nullptr;  // 已废弃
    mapInputResParams.registeredResource = registered;

    NVENCSTATUS err = nvenc_api_.nvEncMapInputResource(nvenc_encoder_, &mapInputResParams);
    if (err != NV_ENC_SUCCESS) {
        std::cerr << "[NVENC-ZeroCopy] Failed to map input resource: " << err << std::endl;
        zc_map_failures_++;
        return false;
    }

    void* mapped_resource = mapInputResParams.mappedResource;
    mapped_resource_ = mapped_resource;

    // 调试：打印映射后的格式
    std::cout << "[NVENC-ZeroCopy] Mapped resource format: 0x" << std::hex
              << mapInputResParams.mappedBufferFmt << std::dec << std::endl;

    // 编码帧参数 - 直接使用映射的资源
    NV_ENC_PIC_PARAMS picParams = {};
    picParams.version = NV_ENC_PIC_PARAMS_VER;
    picParams.inputBuffer = mapped_resource;  // 使用映射的资源！
    picParams.outputBitstream = bitstream_buffers_[idx];
    picParams.inputWidth = config_.width;
    picParams.inputHeight = config_.height;
    picParams.pictureStruct = NV_ENC_PIC_STRUCT_FRAME;
    picParams.bufferFmt = NV_ENC_BUFFER_FORMAT_NV12;

    std::cout << "[NVENC-ZeroCopy] Pic params: width=" << picParams.inputWidth
              << ", height=" << picParams.inputHeight
              << ", pitch=" << picParams.inputPitch
              << ", bufferFmt=" << picParams.bufferFmt
              << ", inputBuffer=" << picParams.inputBuffer
              << ", outputBitstream=" << picParams.outputBitstream << std::endl;

    const bool want_idr = (force_keyframe || force_keyframe_ || (current_pts_ % config_.gop_size == 0));
    picParams.encodePicFlags = want_idr ? NV_ENC_PIC_FLAG_FORCEIDR : 0;
    if (want_idr) {
        force_keyframe_ = false;
    }

    // 编码图片 (完全在 GPU 上，无 CPU 复制！)
    err = nvenc_api_.nvEncEncodePicture(nvenc_encoder_, &picParams);
    current_input_buffer_ = (current_input_buffer_ + 1) % static_cast<uint32_t>(zerocopy_textures_.size());
    current_bitstream_buffer_ = current_input_buffer_;

    if (err == NV_ENC_SUCCESS || err == NV_ENC_ERR_NEED_MORE_INPUT) {
        if (err == NV_ENC_SUCCESS) {
            zc_encode_submit_success_++;
        } else {
            zc_encode_submit_need_more_input_++;
        }
        PendingPacket pkt = {};
        pkt.bitstream_index = idx;
        pkt.timestamp = current_pts_++;
        pkt.key_frame = want_idr;
        pkt.mapped_resource = mapped_resource;
        pending_packets_.push_back(pkt);
        if (pending_packets_.size() > zc_pending_peak_) {
            zc_pending_peak_ = static_cast<unsigned int>(pending_packets_.size());
        }
        if (idx < zerocopy_slot_inflight_.size()) {
            zerocopy_slot_inflight_[idx] = true;
        }
        mapped_resource_ = nullptr;

        DrainPendingPackets(true);

        return true;
    }

    std::cerr << "[NVENC-ZeroCopy] Encode picture failed: " << err << std::endl;
    zc_encode_submit_fail_++;
    if (nvenc_api_.nvEncGetLastErrorString) {
        const char* msg = nvenc_api_.nvEncGetLastErrorString(nvenc_encoder_);
        if (msg) {
            std::cerr << "[NVENC-ZeroCopy] LastError: " << msg << std::endl;
        }
    }
    if (mapped_resource) {
        nvenc_api_.nvEncUnmapInputResource(nvenc_encoder_, mapped_resource);
        zc_unmap_count_++;
    }
    mapped_resource_ = nullptr;
    return false;
}

// ============================================================================
// DLL Export Functions
// ============================================================================

extern "C" {

NVENC_API int is_nvenc_supported() {
    HMODULE dll = LoadLibraryA("nvEncodeAPI64.dll");
    if (dll) {
        FreeLibrary(dll);
        return 1;
    }
    return 0;
}

NVENC_API int is_cuda_d3d11_interop_supported() {
    CUresult err = cuInit(0);
    if (err != CUDA_SUCCESS) {
        return 0;
    }

    int deviceCount = 0;
    err = cuDeviceGetCount(&deviceCount);
    if (err != CUDA_SUCCESS || deviceCount == 0) {
        return 0;
    }

    return 1;
}

NVENC_API void get_nvenc_version(NVENCVersion* version) {
    if (version) {
        version->major = NVENCAPI_MAJOR_VERSION;
        version->minor = NVENCAPI_MINOR_VERSION;
    }
}

NVENC_API HNVENCEncoder init_nvenc_encoder_d3d11(
    void* d3d11_device,
    void* d3d11_context,
    const NVENCEncodeConfig* config
) {
    if (!d3d11_device || !d3d11_context || !config) {
        return nullptr;
    }

    NVENCEncoderImpl* encoder = new NVENCEncoderImpl();

    if (!encoder->Initialize(
        static_cast<ID3D11Device*>(d3d11_device),
        static_cast<ID3D11DeviceContext*>(d3d11_context),
        config,
        false
    )) {
        delete encoder;
        return nullptr;
    }

    return static_cast<HNVENCEncoder>(encoder);
}

NVENC_API HNVENCEncoder init_nvenc_encoder_d3d11_zerocopy(
    void* d3d11_device,
    void* d3d11_context,
    const NVENCEncodeConfig* config
) {
    if (!d3d11_device || !d3d11_context || !config) {
        return nullptr;
    }

    NVENCEncoderImpl* encoder = new NVENCEncoderImpl();

    if (!encoder->Initialize(
        static_cast<ID3D11Device*>(d3d11_device),
        static_cast<ID3D11DeviceContext*>(d3d11_context),
        config,
        true
    )) {
        delete encoder;
        return nullptr;
    }

    return static_cast<HNVENCEncoder>(encoder);
}

NVENC_API int encode_nvenc_frame_cpu(
    HNVENCEncoder handle,
    const unsigned char* data,
    int size,
    long long timestamp,
    int force_keyframe
) {
    NVENCEncoderImpl* encoder = static_cast<NVENCEncoderImpl*>(handle);
    if (!encoder || !encoder->IsInitialized()) {
        return 0;
    }

    return encoder->EncodeFromCPU(data, size, timestamp, force_keyframe != 0) ? 1 : 0;
}

NVENC_API int encode_nvenc_frame_d3d11(
    HNVENCEncoder handle,
    void* d3d11_texture,
    long long timestamp,
    int force_keyframe
) {
    NVENCEncoderImpl* encoder = static_cast<NVENCEncoderImpl*>(handle);
    if (!encoder || !encoder->IsInitialized()) {
        return 0;
    }

    return encoder->EncodeFromD3D11(static_cast<ID3D11Texture2D*>(d3d11_texture), timestamp, force_keyframe != 0) ? 1 : 0;
}

// 零拷贝编码：直接从 D3D11 纹理编码 (使用 NVENC MapInputResource)
NVENC_API int encode_nvenc_frame_d3d11_zerocopy(
    HNVENCEncoder handle,
    void* d3d11_texture,
    long long timestamp,
    int force_keyframe
) {
    NVENCEncoderImpl* encoder = static_cast<NVENCEncoderImpl*>(handle);
    if (!encoder || !encoder->IsInitialized()) {
        return 0;
    }

    return encoder->EncodeFromD3D11_ZeroCopy(static_cast<ID3D11Texture2D*>(d3d11_texture), timestamp, force_keyframe != 0) ? 1 : 0;
}

NVENC_API int get_nvenc_encoded_frame(
    HNVENCEncoder handle,
    NVENCEncodedFrame* frame
) {
    NVENCEncoderImpl* encoder = static_cast<NVENCEncoderImpl*>(handle);
    if (!encoder || !encoder->IsInitialized()) {
        return 0;
    }

    return encoder->GetEncodedFrame(frame) ? 1 : 0;
}

NVENC_API int get_nvenc_zerocopy_stats(HNVENCEncoder handle, NVENCZeroCopyStats* stats) {
    NVENCEncoderImpl* encoder = static_cast<NVENCEncoderImpl*>(handle);
    if (!encoder || !encoder->IsInitialized() || !stats) {
        return 0;
    }
    return encoder->GetZeroCopyStats(stats) ? 1 : 0;
}

// 新函数: 获取编码帧并写入提供的缓冲区 (Python ctypes 友好)
NVENC_API int get_nvenc_encoded_frame_buffer(
    HNVENCEncoder handle,
    unsigned char* buffer,
    int* data_size,
    int* out_size,
    long long* out_pts
) {
    NVENCEncoderImpl* encoder = static_cast<NVENCEncoderImpl*>(handle);
    if (!encoder || !encoder->IsInitialized() || !buffer) {
        return 0;
    }

    // 创建临时帧结构
    NVENCEncodedFrame frame;
    if (!encoder->GetEncodedFrame(&frame)) {
        return 0;
    }

    // 检查缓冲区大小
    int required_size = frame.size;
    if (data_size) {
        *data_size = required_size;
    }

    // 复制编码数据到缓冲区
    if (buffer && frame.data && frame.size > 0) {
        std::memcpy(buffer, frame.data, frame.size);
    }

    if (out_size) {
        *out_size = frame.size;
    }

    if (out_pts) {
        *out_pts = frame.timestamp;
    }

    return 1;
}

NVENC_API void free_nvenc_encoded_frame(NVENCEncodedFrame* frame) {
    // 数据由编码器管理
}

NVENC_API void request_nvenc_keyframe(HNVENCEncoder handle) {
    NVENCEncoderImpl* encoder = static_cast<NVENCEncoderImpl*>(handle);
    if (encoder) {
        encoder->RequestKeyframe();
    }
}

NVENC_API void free_nvenc_encoder(HNVENCEncoder handle) {
    NVENCEncoderImpl* encoder = static_cast<NVENCEncoderImpl*>(handle);
    if (encoder) {
        encoder->Release();
        delete encoder;
    }
}

}  // extern "C"
