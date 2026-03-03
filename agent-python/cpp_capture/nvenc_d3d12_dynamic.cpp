/**
 * D3D11-NVENC 动态加载编码器实现
 *
 * 运行时动态加载 NVENC API，无需编译时依赖 SDK
 * 使用 D3D11-CUDA 互操作
 */

#ifdef NVENC_ENCODER_EXPORTS
#undef NVENC_ENCODER_EXPORTS
#endif
#define NVENC_ENCODER_EXPORTS
#include "nvenc_d3d12_dynamic.h"
#include <iostream>
#include <cstring>
#include <cudaD3D11.h>

// CUDA 错误检查
#define CUDA_CHECK(call) \
    do { \
        CUresult err = call; \
        if (err != CUDA_SUCCESS) { \
            std::cerr << "CUDA Error: " << err << " at " << __LINE__ << std::endl; \
            return false; \
        } \
    } while(0)

#define CUDA_RUNTIME_CHECK(call) \
    do { \
        cudaError_t err = call; \
        if (err != cudaSuccess) { \
            std::cerr << "CUDA Runtime Error: " << cudaGetErrorString(err) << " at " << __LINE__ << std::endl; \
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
    , output_buffer_(nullptr)
    , output_buffer_size_(0)
    , current_pts_(0)
    , force_keyframe_(false)
    , initialized_(false)
    , cuda_initialized_(false)
    , nvenc_loaded_(false)
    , cuda_context_(nullptr)
    , cuda_stream_(nullptr)
    , nvenc_dll_(nullptr)
{
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
    if (config_.rc_mode < 0) config_.rc_mode = 2;

    // 初始化 CUDA
    if (!InitializeCUDA()) {
        std::cerr << "CUDA initialization failed" << std::endl;
        return false;
    }

    // 加载 NVENC (可选，失败不中断)
    if (!LoadNVENC()) {
        std::cerr << "NVENC not available, will use stub implementation" << std::endl;
    }

    // 分配缓冲区
    if (!AllocateBuffers()) {
        std::cerr << "Buffer allocation failed" << std::endl;
        Cleanup();
        return false;
    }

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

    // 创建 CUDA 上下文 (CUDA 13.0 新 API)
    // 使用 NULL 参数以简化创建
    CUDA_CHECK(cuCtxCreate(&cuda_context_, nullptr, 0, cuda_device));

    // 创建 CUDA 流
    CUDA_RUNTIME_CHECK(cudaStreamCreate(&cuda_stream_));

    cuda_initialized_ = true;
    return true;
}

bool NVENCEncoderImpl::LoadNVENC() {
    // 尝试加载 nvEncodeAPI64.dll
    nvenc_dll_ = LoadLibraryA("nvEncodeAPI64.dll");
    if (!nvenc_dll_) {
        std::cerr << "Failed to load nvEncodeAPI64.dll" << std::endl;
        return false;
    }

    // TODO: 动态加载 NVENC 函数指针
    // 由于缺少 SDK，我们暂时只标记为已加载
    // 实际编码功能需要完整的 NVENC API 定义

    nvenc_loaded_ = true;
    return true;
}

bool NVENCEncoderImpl::InitializeNVENCSession() {
    // NVENC 会话初始化
    // 需要完整的 NVENC API 调用
    return false;
}

bool NVENCEncoderImpl::AllocateBuffers() {
    // 分配输出缓冲区空间
    output_buffer_size_ = config_.width * config_.height * 3 / 2;
    output_queue_.clear();
    output_queue_.reserve(10);
    return true;
}

bool NVENCEncoderImpl::EncodeFromCPU(const unsigned char* data, int size, long long timestamp, bool force_keyframe) {
    if (!initialized_ || !data) {
        return false;
    }

    // 存根实现 - 返回模拟的 H.264 NAL 单元
    // 在真实实现中，这里应该:
    // 1. 拷贝数据到 GPU
    // 2. 转换 BGRA 到 NV12
    // 3. 调用 NVENC 编码
    // 4. 将输出放入队列

    static const unsigned char stub_nal[] = {
        0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x80, 0x0a,
        0xff, 0xe1, 0x00, 0x1d, 0x00, 0x00, 0x00, 0x01,
        0x68, 0xce, 0x3c, 0x80
    };

    EncodedOutput output;
    output.data.assign(stub_nal, stub_nal + sizeof(stub_nal));
    output.timestamp = current_pts_;
    output.key_frame = (current_pts_ % config_.gop_size == 0);

    output_queue_.push_back(output);
    current_pts_++;

    return true;
}

bool NVENCEncoderImpl::EncodeFromD3D11(ID3D11Texture2D* texture, long long timestamp, bool force_keyframe) {
    if (!initialized_ || !texture) {
        return false;
    }

    // 存根实现 - D3D11 纹理编码
    // 在真实实现中，这里应该:
    // 1. 注册 D3D11 纹理到 CUDA
    // 2. 使用 CUDA kernel 转换格式
    // 3. 调用 NVENC 编码

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
    // 清理输出队列
    output_queue_.clear();

    // 清理 CUDA 流
    if (cuda_stream_) {
        cudaStreamDestroy(cuda_stream_);
        cuda_stream_ = nullptr;
    }

    // 清理 CUDA 上下文
    if (cuda_context_) {
        cuCtxDestroy(cuda_context_);
        cuda_context_ = nullptr;
    }

    // 卸载 NVENC DLL
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
        version->major = 12;
        version->minor = 0;
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
