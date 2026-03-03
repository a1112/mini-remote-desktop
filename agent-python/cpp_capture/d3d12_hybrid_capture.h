/**
 * D3D12 混合捕获 - Desktop Duplication + D3D11On12
 *
 * 实用方案: D3D11 捕获 → D3D12 资源
 *
 * 优势:
 * - 成熟的 Desktop Duplication API
 * - D3D12 输出用于编码器集成
 * - 零额外拷贝 (共享资源)
 */
#pragma once

#include <windows.h>
#include <d3d11.h>
#include <d3d11_2.h>
#include <d3d12.h>
#include <dxgi1_6.h>
#include <wrl/client.h>

using Microsoft::WRL::ComPtr;

// 前向声明
class D3D12HybridCapturer;

// D3D12 帧数据
struct D3D12HybridFrame {
    int width;
    int height;
    int stride;
    int format;  // DXGI_FORMAT as int
    unsigned long long timestamp;
    void* d3d11_resource;   // ID3D11Texture2D*
    void* d3d12_resource;   // ID3D12Resource* (可选)
};

// 捕获句柄
typedef void* HD3D12HybridCapture;

// 导出宏
#ifdef D3D12_HYBRID_CAPTURE_EXPORTS
#define EXPORT_API __declspec(dllexport)
#else
#define EXPORT_API __declspec(dllimport)
#endif

#ifdef __cplusplus
extern "C" {
#endif

/**
 * 初始化混合捕获器
 */
EXPORT_API HD3D12HybridCapture init_hybrid_capture(int monitor_index, int enable_d3d12);

/**
 * 捕获一帧
 */
EXPORT_API int capture_hybrid_frame(
    HD3D12HybridCapture handle,
    D3D12HybridFrame* frame_info
);

/**
 * 复制帧到 CPU 缓冲区
 */
EXPORT_API int copy_hybrid_frame_to_cpu(
    HD3D12HybridCapture handle,
    unsigned char* buffer,
    int buffer_size
);

/**
 * 获取 D3D12 设备 (用于编码器集成)
 */
EXPORT_API void* get_hybrid_d3d12_device(HD3D12HybridCapture handle);

/**
 * 获取 D3D12 命令队列
 */
EXPORT_API void* get_hybrid_d3d12_queue(HD3D12HybridCapture handle);

/**
 * 释放捕获器
 */
EXPORT_API void free_hybrid_capture(HD3D12HybridCapture handle);

/**
 * 获取 D3D11 资源 (用于回退)
 */
EXPORT_API void* get_hybrid_d3d11_resource(HD3D12HybridCapture handle);

#ifdef __cplusplus
}
#endif


// 内部实现
class D3D12HybridCapturer {
public:
    D3D12HybridCapturer();
    ~D3D12HybridCapturer();

    bool Initialize(int monitor_index, bool enable_d3d12);
    bool Capture(D3D12HybridFrame* frame_info);
    bool CopyToCPU(unsigned char* buffer, int buffer_size);
    void Release();

    ID3D12Device* GetD3D12Device() const { return d3d12_device_.Get(); }
    ID3D12CommandQueue* GetD3D12Queue() const { return d3d12_queue_.Get(); }
    ID3D11Texture2D* GetD3D11Resource() const { return captured_texture_d3d11_.Get(); }
    bool HasD3D12() const { return d3d12_enabled_; }
    bool IsInitialized() const { return initialized_; }

private:
    bool CreateD3D11Device();
    bool CreateD3D12Device();
    bool InitializeDesktopDuplication();
    bool CreateSharedResources();
    bool AcquireNextFrame();
    bool CopyToSharedResources();
    void ReleaseFrame();

    // D3D11 组件
    ComPtr<ID3D11Device> d3d11_device_;
    ComPtr<ID3D11DeviceContext> d3d11_context_;
    ComPtr<IDXGIOutputDuplication> duplication_;
    ComPtr<ID3D11Texture2D> captured_texture_d3d11_;
    ComPtr<ID3D11Texture2D> staging_texture_;

    // D3D12 组件
    bool d3d12_enabled_ = false;
    ComPtr<ID3D12Device> d3d12_device_;
    ComPtr<ID3D12CommandQueue> d3d12_queue_;
    ComPtr<ID3D12CommandAllocator> d3d12_allocator_;
    ComPtr<ID3D12GraphicsCommandList> d3d12_list_;

    // 共享资源
    ComPtr<ID3D11Texture2D> shared_texture_d3d11_;
    ComPtr<ID3D12Resource> shared_texture_d3d12_;
    HANDLE shared_handle_;

    // 状态
    int width_ = 0;
    int height_ = 0;
    bool initialized_ = false;
    DXGI_OUTDUPL_FRAME_INFO frame_info_;
    ComPtr<IDXGIResource> resource_;
};
