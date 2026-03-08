/**
 * D3D12 Screen Capture using Windows.Graphics.Capture API
 *
 * 纯 D3D12 实现 - 适用于 Windows 10 1803+
 *
 * 优势:
 * - 原生 D3D12 资源输出
 * - 与 D3D12 编码器零拷贝集成
 * - 支持多显示器和高刷新率
 *
 * 编译: 需要 Windows SDK 10.0.17763.0+
 */
#pragma once

#include <windows.h>
#include <d3d12.h>
#include <dxgi1_6.h>
#include <wrl/client.h>
#include <mutex>
#include <memory>

// Windows.Graphics.Capture 相关接口
#include <DispatcherQueue.h>
#include <Windows.Graphics.Capture.Interop.h>
#include <Windows.Graphics.DirectX.Direct3D11.Interop.h>
#include <Windows.Graphics.DirectX.Direct3D12.Interop.h>

using Microsoft::WRL::ComPtr;

// 帧数据结构
struct D3D12FrameInfo {
    int width;
    int height;
    int stride;
    DXGI_FORMAT format;
    UINT64 timestamp;
    void* gpu_resource;  // ID3D12Resource* 指针
};

// 捕获句柄
typedef void* HD3D12Capture;

#ifdef __cplusplus
extern "C" {
#endif

/**
 * 初始化 D3D12 捕获器
 *
 * @param monitor_index 显示器索引 (0 = 主显示器)
 * @param gpu_index GPU 索引 (默认 0)
 * @return 捕获句柄，NULL 表示失败
 */
HD3D12Capture __declspec(dllexport) init_d3d12_capture(int monitor_index, int gpu_index);

/**
 * 捕获一帧到 D3D12 资源
 *
 * @param handle 捕获句柄
 * @param pp_output_resource 输出 D3D12 资源 (调用者不负责释放)
 * @param info 输出帧信息
 * @return 1 成功, 0 失败, -1 需要重试
 */
int __declspec(dllexport) capture_d3d12_frame(
    HD3D12Capture handle,
    void** pp_output_resource,
    D3D12FrameInfo* info
);

/**
 * 捕获一帧到 CPU 缓冲区 (用于测试/兼容)
 *
 * @param handle 捕获句柄
 * @param buffer 输出缓冲区 (BGRA 格式)
 * @param buffer_size 缓冲区大小
 * @param info 输出帧信息
 * @return 1 成功, 0 失败, -1 需要重试
 */
int __declspec(dllexport) capture_d3d12_to_cpu(
    HD3D12Capture handle,
    unsigned char* buffer,
    int buffer_size,
    D3D12FrameInfo* info
);

/**
 * 获取 D3D12 设备 (用于与其他 D3D12 组件集成)
 *
 * @param handle 捕获句柄
 * @return ID3D12Device* 指针
 */
void* __declspec(dllexport) get_d3d12_device(HD3D12Capture handle);

/**
 * 获取命令队列 (用于编码器集成)
 *
 * @param handle 捕获句柄
 * @return ID3D12CommandQueue* 指针
 */
void* __declspec(dllexport) get_d3d12_command_queue(HD3D12Capture handle);

/**
 * 释放捕获器
 */
void __declspec(dllexport) free_d3d12_capture(HD3D12Capture handle);

/**
 * 检查系统是否支持 D3D12 捕获
 *
 * @return 1 支持, 0 不支持
 */
int __declspec(dllexport) is_d3d12_capture_supported();

/**
 * 获取支持的捕获方式
 *
 * @return 位掩码: 1=DesktopDuplication, 2=GraphicsCapture, 4=Both
 */
int __declspec(dllexport) get_supported_capture_methods();

#ifdef __cplusplus
}
#endif


// 内部实现类
class D3D12Capturer {
public:
    D3D12Capturer();
    ~D3D12Capturer();

    bool Initialize(int monitor_index, int gpu_index);
    bool CaptureToD3D12Resource(ID3D12Resource** pp_resource, D3D12FrameInfo* info);
    bool CaptureToCPU(unsigned char* buffer, int buffer_size, D3D12FrameInfo* info);
    void Release();

    ID3D12Device* GetDevice() const { return device_.Get(); }
    ID3D12CommandQueue* GetCommandQueue() const { return command_queue_.Get(); }
    int GetWidth() const { return width_; }
    int GetHeight() const { return height_; }
    bool IsInitialized() const { return initialized_; }

private:
    bool CreateD3D12Device(int gpu_index);
    bool CreateCommandQueue();
    bool CreateFramePool();
    bool StartCaptureSession();
    bool ProcessFrame();

    // D3D12 组件
    ComPtr<ID3D12Device> device_;
    ComPtr<ID3D12CommandQueue> command_queue_;
    ComPtr<ID3D12CommandAllocator> command_allocator_;
    ComPtr<ID3D12GraphicsCommandList> command_list_;
    ComPtr<ID3D12Resource> captured_resource_;
    ComPtr<ID3D12Resource> staging_resource_;

    // Windows.Graphics.Capture 组件
    ComPtr<IGraphicsCaptureItem> capture_item_;
    ComPtr<IDirect3D12CaptureFramePool> frame_pool_;
    ComPtr<IGraphicsCaptureSession> capture_session_;

    // 状态
    int width_ = 0;
    int height_ = 0;
    bool initialized_ = false;
    std::mutex mutex_;

    // GPU 同步
    HANDLE fence_event_ = NULL;
    ComPtr<ID3D12Fence> fence_;
    UINT64 fence_value_ = 0;
};
