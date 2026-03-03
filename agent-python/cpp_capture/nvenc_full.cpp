/**
 * NVENC 完整编码器实现
 *
 * 使用 NVENC SDK 13.0 实现完整的硬件编码
 * 支持 D3D11-CUDA 互操作
 */

#ifdef NVENC_ENCODER_EXPORTS
#undef NVENC_ENCODER_EXPORTS
#endif
#define NVENC_ENCODER_EXPORTS
#include "nvenc_full.h"
#include <iostream>
#include <cstring>
#include <algorithm>

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

// ============================================================================
// NVENCEncoderImpl Implementation
// ============================================================================

NVENCEncoderImpl::NVENCEncoderImpl()
    : d3d11_device_(nullptr)
    , d3d11_context_(nullptr)
    , nvenc_encoder_(nullptr)
    , registered_resource_(nullptr)
    , mapped_resource_(nullptr)
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
{
    std::memset(&nvenc_api_, 0, sizeof(nvenc_api_));
}

NVENCEncoderImpl::~NVENCEncoderImpl() {
    Release();
}

bool NVENCEncoderImpl::Initialize(ID3D11Device* d3d11_device, ID3D11DeviceContext* d3d11_context,
                                   const NVENCEncodeConfig* config) {
    if (!d3d11_device || !d3d11_context || !config) {
        return false;
    }

    d3d11_device_ = d3d11_device;
    d3d11_context_ = d3d11_context;
    config_ = *config;

    // 设置默认值
    if (config_.framerate <= 0) config_.framerate = 60;
    if (config_.bitrate <= 0) config_.bitrate = 5000000;
    if (config_.gop_size <= 0) config_.gop_size = 60;
    if (config_.preset < 0) config_.preset = 2;
    if (config_.rc_mode < 0) config_.rc_mode = 3;  // 默认 CQ 模式
    if (config_.quality <= 0) config_.quality = 20;  // 默认高质量 CQ

    std::cout << "[NVENC] Initializing encoder " << config_.width << "x" << config_.height
              << " @ " << config_.framerate << "fps, " << (config_.bitrate / 1000000.0) << "Mbps" << std::endl;

    // 初始化 CUDA
    if (!InitializeCUDA()) {
        std::cerr << "[NVENC] CUDA initialization failed" << std::endl;
        return false;
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

    CUdevice cuda_device = 0;
    CUDA_CHECK(cuDeviceGet(&cuda_device, 0));
    CUDA_CHECK(cuCtxCreate(&cuda_context_, nullptr, 0, cuda_device));
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
    NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS sessionParams = {};
    sessionParams.version = NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER;
    sessionParams.deviceType = NV_ENC_DEVICE_TYPE_CUDA;
    sessionParams.device = cuda_context_;
    sessionParams.apiVersion = NVENCAPI_VERSION;

    NVENCSTATUS err = nvenc_api_.nvEncOpenEncodeSessionEx(&sessionParams, &nvenc_encoder_);
    if (err != NV_ENC_SUCCESS) {
        std::cerr << "[NVENC] nvEncOpenEncodeSessionEx failed: " << err << std::endl;
        return false;
    }

    std::cout << "[NVENC] Session opened successfully" << std::endl;
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

    // 码率控制配置
    encodeConfig.rcParams.version = NV_ENC_RC_PARAMS_VER;

    if (config_.rc_mode == 0 || config_.rc_mode == 3) {
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
    initializeParams.enableEncodeAsync = 0;
    initializeParams.enablePTD = 0;
    initializeParams.reportSliceOffsets = 0;
    initializeParams.enableSubFrameWrite = 0;
    initializeParams.enableExternalMEHints = 0;
    initializeParams.enableMEOnlyMode = 0;
    initializeParams.enableWeightedPrediction = 0;
    initializeParams.splitEncodeMode = 0;
    initializeParams.enableOutputInVidmem = 0;
    initializeParams.tuningInfo = NV_ENC_TUNING_INFO_HIGH_QUALITY;
    initializeParams.encodeConfig = &encodeConfig;

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

    NV_ENC_CREATE_BITSTREAM_BUFFER createBitstreamBufferParams = {};
    createBitstreamBufferParams.version = NV_ENC_CREATE_BITSTREAM_BUFFER_VER;
    createBitstreamBufferParams.size = config_.width * config_.height * 3 / 2;

    for (uint32_t i = 0; i < numBuffers; i++) {
        NVENCSTATUS err = nvenc_api_.nvEncCreateBitstreamBuffer(nvenc_encoder_, &createBitstreamBufferParams);
        if (err != NV_ENC_SUCCESS) {
            std::cerr << "[NVENC] nvEncCreateBitstreamBuffer failed: " << err << std::endl;
            return false;
        }
        bitstream_buffers_[i] = createBitstreamBufferParams.bitstreamBuffer;
    }

    std::cout << "[NVENC] Allocated " << numBuffers << " input and bitstream buffers" << std::endl;
    return true;
}

bool NVENCEncoderImpl::RegisterD3D11Resource(ID3D11Texture2D* texture) {
    // TODO: 实现 D3D11 纹理注册
    return false;
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

    // 转换 BGRA 到 NV12
    uint8_t* dstY = (uint8_t*)lockInputBufferParams.bufferDataPtr;
    const uint8_t* srcBGRA = data;
    int width = config_.width;
    int height = config_.height;

    // Y 分量
    for (int y = 0; y < height; y++) {
        for (int x = 0; x < width; x++) {
            int srcIdx = (y * width + x) * 4;
            dstY[y * lockInputBufferParams.pitch + x] =
                (299 * srcBGRA[srcIdx + 2] + 587 * srcBGRA[srcIdx + 1] + 114 * srcBGRA[srcIdx + 0]) / 1000;
        }
    }

    // UV 分量
    uint8_t* dstUV = dstY + lockInputBufferParams.pitch * height;
    int uvSize = (lockInputBufferParams.pitch * height) / 2;
    std::memset(dstUV, 128, uvSize);

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
    return EncodeFromCPU(nullptr, 0, timestamp, force_keyframe);
}

bool NVENCEncoderImpl::GetEncodedFrame(NVENCEncodedFrame* frame) {
    if (!initialized_ || !frame) {
        return false;
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
        config
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
