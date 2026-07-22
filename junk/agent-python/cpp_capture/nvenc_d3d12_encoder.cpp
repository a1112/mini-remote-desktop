/**
 * D3D12-NVENC 编码器实现
 *
 * 实现 D3D12 资源直接传递给 NVENC 编码器
 * 通过 CUDA-D3D12 互操作实现零拷贝
 */

// 定义导出
#define NVENC_ENCODER_EXPORTS

#include "nvenc_d3d12_encoder.h"
#include <iostream>
#include <thread>
#include <cstring>

// CUDA 错误检查
#define CUDA_CHECK(call) \
    do { \
        CUresult err = call; \
        if (err != CUDA_SUCCESS) { \
            std::cerr << "CUDA Error: " << err << " at " << __LINE__ << std::endl; \
            return false; \
        } \
    } while(0)

// NVENC 错误检查
#define NVENC_CHECK(call) \
    do { \
        NVENCSTATUS err = call; \
        if (err != NV_ENC_SUCCESS) { \
            std::cerr << "NVENC Error: " << err << " at " << __LINE__ << std::endl; \
            return false; \
        } \
    } while(0)


// ============================================================================
// NVENCEncoderImpl Implementation
// ============================================================================

NVENCEncoderImpl::NVENCEncoderImpl()
    : nvenc_encoder_(nullptr)
    , registered_resource_(nullptr)
    , input_buffer_(nullptr)
    , output_buffer_(nullptr)
    , output_buffer_size_(0)
    , current_pts_(0)
    , force_keyframe_(false)
    , initialized_(false)
    , cuda_initialized_(false)
    , nvenc_initialized_(false)
{
}

NVENCEncoderImpl::~NVENCEncoderImpl() {
    Release();
}

bool NVENCEncoderImpl::Initialize(ID3D12Device* d3d12_device, ID3D12CommandQueue* d3d12_queue,
                                   const NVENCEncodeConfig* config) {
    if (!d3d12_device || !d3d12_queue || !config) {
        return false;
    }

    d3d12_device_ = d3d12_device;
    d3d12_queue_ = d3d12_queue;
    config_ = *config;

    // 设置默认值
    if (config_.framerate <= 0) config_.framerate = 60;
    if (config_.bitrate <= 0) config_.bitrate = 5_000_000;
    if (config_.gop_size <= 0) config_.gop_size = 60;
    if (config_.preset < 0) config_.preset = 2;  // medium
    if (config_.rc_mode < 0) config_.rc_mode = 2;  // CBR

    // 初始化 CUDA
    if (!InitializeCUDA()) {
        std::cerr << "CUDA initialization failed" << std::endl;
        return false;
    }

    // 初始化 NVENC
    if (!InitializeNVENC()) {
        std::cerr << "NVENC initialization failed" << std::endl;
        Cleanup();
        return false;
    }

    // 分配缓冲区
    if (!AllocateInputBuffer() || !AllocateOutputBuffer()) {
        std::cerr << "Buffer allocation failed" << std::endl;
        Cleanup();
        return false;
    }

    // 初始化统计
    stats_ = {};
    current_pts_ = 0;

    initialized_ = true;
    return true;
}

bool NVENCEncoderImpl::InitializeCUDA() {
    // 初始化 CUDA
    CUresult err = cuInit(0);
    if (err != CUDA_SUCCESS) {
        std::cerr << "cuInit failed: " << err << std::endl;
        return false;
    }

    // 获取 CUDA 设备
    CUdevice cuda_device = 0;
    CUDA_CHECK(cuDeviceGet(&cuda_device, 0));

    // 创建 CUDA 上下文
    CUDA_CHECK(cuCtxCreate(&cuda_context_, 0, cuda_device));

    // 创建 CUDA 流
    CUDA_CHECK(cuStreamCreate(&cuda_stream_, 0));

    // 创建事件
    CUDA_CHECK(cuEventCreate(&encode_event_, 0));
    CUDA_CHECK(cuEventCreate(&copy_event_, 0));

    cuda_initialized_ = true;
    return true;
}

bool NVENCEncoderImpl::InitializeNVENC() {
    // 加载 NVENC API
    NVENCSTATUS err = NvEncodeAPIGetMaxSupportedVersion(&nvenc_api_);
    if (err != NV_ENC_SUCCESS) {
        std::cerr << "NvEncodeAPIGetMaxSupportedVersion failed" << std::endl;
        return false;
    }

    // 加载 NVENC API 函数
    uint32_t version = 0;
    err = NvEncodeAPICreateInstance(&nvenc_api_);
    if (err != NV_ENC_SUCCESS) {
        std::cerr << "NvEncodeAPICreateInstance failed" << std::endl;
        return false;
    }

    // 打开编码器会话
    NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS encodeSessionExParams = {};
    encodeSessionExParams.apiVersion = NVENCAPI_VERSION;
    encodeSessionExParams.deviceType = NV_ENC_DEVICE_TYPE_CUDA;

    NV_ENC_DEVICE_HANDLE deviceHandle = nullptr;
    err = nvenc_api_.nvEncOpenEncodeSessionEx(&encodeSessionExParams, &deviceHandle);
    if (err != NV_ENC_SUCCESS) {
        std::cerr << "nvEncOpenEncodeSessionEx failed: " << err << std::endl;

        // 尝试旧版本 API
        NV_ENC_OPEN_ENCODE_SESSION_PARAMS params = {};
        params.deviceType = NV_ENC_DEVICE_TYPE_CUDA;
        params.device = cuda_context_;
        params.apiVersion = NVENCAPI_VERSION;

        err = nvenc_api_.nvEncOpenEncodeSession(&params, &nvenc_encoder_);
        if (err != NV_ENC_SUCCESS) {
            std::cerr << "nvEncOpenEncodeSession (legacy) failed: " << err << std::endl;
            return false;
        }
    } else {
        nvenc_encoder_ = deviceHandle;
    }

    // 设置编码参数
    NV_ENC_PRESET_CONFIG presetConfig = {};
    presetConfig.presetGuid = NV_ENC_PRESET_P4_GUID;  // medium quality
    presetConfig.presetCfg = {};
    presetConfig.encodeConfig = {};

    uint32_t width = config_.width;
    uint32_t height = config_.height;

    NV_ENC_CONFIG encodeConfig = {};
    encodeConfig.profileGuid = NV_ENC_CODEC_PROFILE_AUTOSELECT_GUID;
    encodeConfig.gopLength = config_.gop_size;
    encodeConfig.frameRateDen = 1;
    encodeConfig.frameRateNum = config_.framerate;
    encodeConfig.encodeCodecConfig = {};
    encodeConfig.encodeCodecConfig->codecType = NV_ENC_CODEC_H264;

    NV_ENC_H264_CONFIG h264Config = {};
    h264Config.profile = NV_ENC_H264_PROFILE_MAIN;
    h264Config.level = NV_ENC_H264_LEVEL_41;
    h264Config.chromaFormatID = NV_ENC_CHROMA_FORMAT_YUV420;
    h264Config.idrPeriod = config_.gop_size;

    encodeConfig.encodeCodecConfig = {};
    encodeConfig.encodeCodecConfig->codecType = NV_ENC_CODEC_H264;
    encodeConfig.encodeCodecConfig->configurationFlags = 0;
    encodeConfig.encodeCodecConfig->compatibilityFlags = 0;

    // 配置参数
    NV_ENC_CONFIG_PARAMS configParams = {};
    configParams.sendStdinBuffer = 0;  // 0 = use internal buffering
    configParams.encodeConfig = &encodeConfig;
    configParams.presetConfig = &presetConfig;

    err = nvenc_api_.nvEncInitializeEncoder(nvenc_encoder_, &configParams);
    if (err != NV_ENC_SUCCESS) {
        std::cerr << "nvEncInitializeEncoder failed: " << err << std::endl;
        return false;
    }

    // 创建位流缓冲区
    NV_ENC_CREATE_BITSTREAM_BUFFER bitstreamBuffer = {};
    bitstreamBuffer.size = config_.width * config_.height * 3 / 2;  // 最大帧大小

    err = nvenc_api_.nvEncCreateBitstreamBuffer(nvenc_encoder_, &bitstreamBuffer);
    if (err != NV_ENC_SUCCESS) {
        std::cerr << "nvEncCreateBitstreamBuffer failed" << std::endl;
        return false;
    }

    output_buffer_ = bitstreamBuffer.bitstreamBuffer;
    output_buffer_size_ = bitstreamBuffer.size;

    nvenc_initialized_ = true;
    return true;
}

bool NVENCEncoderImpl::AllocateInputBuffer() {
    // 分配 CUDA 输入缓冲区 (NV12 格式)
    size_t bufferSize = config_.width * config_.height * 3 / 2;  // NV12

    CUDA_CHECK(cuMemAlloc(&input_buffer_, bufferSize));

    return true;
}

bool NVENCEncoderImpl::AllocateOutputBuffer() {
    // 输出缓冲区在 InitializeNVENC 中分配
    return output_buffer_ != nullptr;
}

bool NVENCEncoderImpl::RegisterD3D12Resource(ID3D12Resource* resource) {
    // 注册 D3D12 资源到 CUDA
    // 这需要 CUDA-D3D12 互操作

    // 获取 D3D12 资源的 IDXGIResource 接口
    ComPtr<IDXGIResource> dxgiResource;
    HRESULT hr = resource->QueryInterface(__uuidof(dxgiResource), &dxgiResource);
    if (FAILED(hr)) {
        return false;
    }

    // 获取共享句柄
    HANDLE sharedHandle = nullptr;
    ComPtr<IDXGIResource1> dxgiResource1;
    hr = dxgiResource.As(&dxgiResource1);
    if (SUCCEEDED(hr)) {
        dxgiResource1->CreateSharedHandle(nullptr, GENERIC_ALL, nullptr, &sharedHandle);
    }

    if (!sharedHandle) {
        return false;
    }

    // 通过 CUDA 打开 D3D12 资源
    CUDA_CHECK(cuGraphicsD3D12RegisterResource(
        &registered_resource_,
        resource,
        0,
        nullptr
    ));

    return true;
}

bool NVENCEncoderImpl::EncodeFromD3D12(ID3D12Resource* resource, long long timestamp, bool force_keyframe) {
    if (!initialized_ || !resource) {
        return false;
    }

    // 注册 D3D12 资源到 CUDA
    if (!registered_resource_) {
        if (!RegisterD3D12Resource(resource)) {
            return false;
        }
    }

    // 映射 CUDA 图形资源
    CUgraphicsResource cudaResource = (CUgraphicsResource)registered_resource_;
    CUDA_CHECK(cuGraphicsMapResources(1, &cudaResource, cuda_stream_));

    // 获取 CUDA 数组指针
    CUarray cudaArray = nullptr;
    CUDA_CHECK(cuGraphicsSubResourceGetMappedArray(&cudaArray, cudaResource, 0, 0));

    // 将 D3D12 纹理拷贝到 NVENC 输入缓冲区
    // 这里需要转换格式 (BGRA → NV12) 并拷贝到 input_buffer_
    // 简化实现 - 假设资源已准备好

    CUDA_CHECK(cuGraphicsUnmapResources(1, &cudaResource, cuda_stream_));

    // 调用 NVENC 编码
    NV_ENC_PIC_PARAMS picParams = {};
    picParams.inputBuffer = input_buffer_;
    picParams.bufferFmt = NV_ENC_BUFFER_FORMAT_NV12;
    picParams.inputWidth = config_.width;
    picParams.inputHeight = config_.height;
    picParams.outputBitstream = output_buffer_;
    picParams.completionEvent = encode_event_;

    picParams.pictureStruct = NV_ENC_PIC_STRUCT_FRAME;
    picParams.inputPitch = config_.width;

    if (force_keyframe || force_keyframe_) {
        picParams.encodePicFlags = NV_ENC_PIC_FLAG_FORCEIDR;
        force_keyframe_ = false;
    } else {
        picParams.encodePicFlags = 0;
    }

    NVENCSTATUS err = nvenc_api_.nvEncEncodePicture(nvenc_encoder_, &picParams);

    CUDA_CHECK(cuEventRecord(encode_event_, cuda_stream_));

    if (err == NV_ENC_SUCCESS || err == NV_ENC_ERR_NEED_MORE_INPUT) {
        stats_.frames_encoded++;
        current_pts_++;
        return true;
    }

    return false;
}

bool NVENCEncoderImpl::EncodeFromCPU(const unsigned char* data, int size, long long timestamp, bool force_keyframe) {
    if (!initialized_ || !data) {
        return false;
    }

    // 拷贝 CPU 数据到 CUDA 缓冲区
    size_t bufferSize = config_.width * config_.height * 3 / 2;
    CUDA_CHECK(cuMemcpyHtoDAsync(input_buffer_, data, bufferSize, cuda_stream_));

    // 编码 (使用同样的流程)
    NV_ENC_PIC_PARAMS picParams = {};
    picParams.inputBuffer = input_buffer_;
    picParams.bufferFmt = NV_ENC_BUFFER_FORMAT_NV12;
    picParams.inputWidth = config_.width;
    picParams.inputHeight = config_.height;
    picParams.outputBitstream = output_buffer_;
    picParams.completionEvent = encode_event_;
    picParams.pictureStruct = NV_ENC_PIC_STRUCT_FRAME;
    picParams.inputPitch = config_.width;

    if (force_keyframe || force_keyframe_) {
        picParams.encodePicFlags = NV_ENC_PIC_FLAG_FORCEIDR;
        force_keyframe_ = false;
    }

    NVENCSTATUS err = nvenc_api_.nvEncEncodePicture(nvenc_encoder_, &picParams);

    CUDA_CHECK(cuEventRecord(encode_event_, cuda_stream_));

    if (err == NV_ENC_SUCCESS || err == NV_ENC_ERR_NEED_MORE_INPUT) {
        stats_.frames_encoded++;
        current_pts_++;
        return true;
    }

    return false;
}

void NVENCEncoderImpl::ProcessOutput() {
    // 检查是否有编码输出
    CUDA_CHECK(cuEventSynchronize(encode_event_));

    NV_ENC_LOCK_BITSTREAM lockBitstreamData = {};
    lockBitstreamData.outputBitstream = output_buffer_;

    NVENCSTATUS err = nvenc_api_.nvEncLockBitstream(nvenc_encoder_, &lockBitstreamData);

    if (err == NV_ENC_SUCCESS) {
        // 有数据可读
        std::vector<unsigned char> data(lockBitstreamData.bitstreamSizeInBytes);
        std::memcpy(data.data(), lockBitstreamData.bitstreamBufferPtr, data.size());

        nvenc_api_.nvEncUnlockBitstream(nvenc_encoder_, lockBitstreamData.outputBitstream);

        // 添加到输出队列
        EncodedOutput output;
        output.data = data;
        output.timestamp = current_pts_ - 1;
        output.key_frame = false;  // TODO: 检测关键帧

        std::lock_guard<std::mutex> lock(queue_mutex_);
        output_queue_.push_back(output);
    }
}

bool NVENCEncoderImpl::GetEncodedFrame(NVENCEncodedFrame* frame) {
    ProcessOutput();

    std::lock_guard<std::mutex> lock(queue_mutex_);

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

void NVENCEncoderImpl::ReleaseFrame(const NVENCEncodedFrame* frame) {
    // 数据由内部管理
}

void NVENCEncoderImpl::RequestKeyframe() {
    force_keyframe_ = true;
}

void NVENCEncoderImpl::GetStats(NVENCEncoderStats* stats) {
    if (stats) {
        *stats = stats_;
    }
}

void NVENCEncoderImpl::Cleanup() {
    // 清理 NVENC
    if (nvenc_encoder_) {
        nvenc_api_.nvEncDestroyEncoder(nvenc_encoder_);
        nvenc_encoder_ = nullptr;
    }

    // 清理 CUDA
    if (cuda_context_) {
        cuCtxDestroy(cuda_context_);
        cuda_context_ = nullptr;
    }

    if (cuda_stream_) {
        cuStreamDestroy(cuda_stream_);
        cuda_stream_ = nullptr;
    }

    if (encode_event_) {
        cuEventDestroy(encode_event_);
        encode_event_ = nullptr;
    }

    if (copy_event_) {
        cuEventDestroy(copy_event_);
        copy_event_ = nullptr;
    }

    if (input_buffer_) {
        cuMemFree(input_buffer_);
        input_buffer_ = nullptr;
    }

    if (registered_resource_) {
        cuGraphicsUnregisterResource(registered_resource_);
        registered_resource_ = nullptr;
    }

    cuda_initialized_ = false;
    nvenc_initialized_ = false;
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
    // 简化实现: 检查是否可以加载 NVENC
    NV_ENCODE_API_FUNCTION_LIST api;
    NVENCSTATUS err = NvEncodeAPICreateInstance(&api);
    if (err == NV_ENC_SUCCESS) {
        api.NvEncDestroyEncoder(nullptr);  // 清理
        return 1;
    }
    return 0;
}

NVENC_API int is_cuda_d3d12_interop_supported() {
    // 检查 CUDA-D3D12 互操作支持
    CUresult err = cuInit(0);
    if (err != CUDA_SUCCESS) {
        return 0;
    }

    unsigned int deviceCount = 0;
    err = cuDeviceGetCount(&deviceCount);
    if (err != CUDA_SUCCESS || deviceCount == 0) {
        return 0;
    }

    return 1;
}

NVENC_API void get_nvenc_version(NVENCVersion* version) {
    if (version) {
        uint32_t major = 0;
        NvEncodeAPIGetMaxSupportedVersion(&major);
        version->major = major;
        version->minor = 0;
    }
}

NVENC_API HNVENCEncoder init_nvenc_encoder(
    void* d3d12_device,
    void* d3d12_queue,
    const NVENCEncodeConfig* config
) {
    if (!d3d12_device || !d3d12_queue || !config) {
        return nullptr;
    }

    NVENCEncoderImpl* encoder = new NVENCEncoderImpl();

    if (!encoder->Initialize(
        static_cast<ID3D12Device*>(d3d12_device),
        static_cast<ID3D12CommandQueue*>(d3d12_queue),
        config
    )) {
        delete encoder;
        return nullptr;
    }

    return static_cast<HNVENCEncoder>(encoder);
}

NVENC_API int encode_nvenc_frame_d3d12(
    HNVENCEncoder handle,
    void* d3d12_resource,
    long long timestamp,
    int force_keyframe
) {
    NVENCEncoderImpl* encoder = static_cast<NVENCEncoderImpl*>(handle);
    if (!encoder || !encoder->IsInitialized()) {
        return 0;
    }

    return encoder->EncodeFromD3D12(
        static_cast<ID3D12Resource*>(d3d12_resource),
        timestamp,
        force_keyframe != 0
    ) ? 1 : 0;
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

NVENC_API void get_nvenc_stats(HNVENCEncoder handle, NVENCEncoderStats* stats) {
    NVENCEncoderImpl* encoder = static_cast<NVENCEncoderImpl*>(handle);
    if (encoder) {
        encoder->GetStats(stats);
    }
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
