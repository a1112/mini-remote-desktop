/**
 * Windows.Graphics.Capture (WGC) API 实现
 *
 * 特性:
 * - 支持屏幕捕获
 * - 支持窗口捕获
 * - GPU Direct (D3D11 纹理输出)
 * - 并发捕获支持
 * - 低延迟 (~1-3ms)
 *
 * 要求: Windows 10 1803+ (Build 17134)
 */

#pragma once

#include <windows.h>
#include <d3d11.h>
#include <wrl/client.h>

using Microsoft::WRL::ComPtr;

// 导出宏
#ifdef WGC_CAPTURE_EXPORTS
#define WGC_API __declspec(dllexport)
#else
#define WGC_API __declspec(dllimport)
#endif

// ============================================================================
// 公共接口
// ============================================================================

#ifdef __cplusplus
extern "C" {
#endif

// 不透明句柄
typedef void* HWgcCapture;

// 捕获类型
typedef enum {
    WGC_CAPTURE_MONITOR = 0,  // 监视器捕获
    WGC_CAPTURE_WINDOW = 1,   // 窗口捕获
} WgcCaptureType;

// 捕获帧信息
typedef struct {
    int width;
    int height;
    int stride;               // 每行字节数
    int format;               // DXGI_FORMAT
    long long timestamp;      // 时间戳
    void* d3d11_texture;      // D3D11 纹理指针 (用于 GPU Direct)
    unsigned int frame_id;    // 帧序号
} WgcFrameInfo;

// 窗口信息
typedef struct {
    HWND hwnd;                // 窗口句柄
    char title[256];          // 窗口标题
    int is_minimized;         // 是否最小化
    int is_visible;           // 是否可见
    int rect_left;            // 窗口位置
    int rect_top;
    int rect_right;
    int rect_bottom;
} WgcWindowInfo;

// 监视器信息
typedef struct {
    int index;                // 监视器索引
    char name[64];            // 监视器名称
    int rect_left;            // 监视器位置
    int rect_top;
    int rect_right;
    int rect_bottom;
    int width;                // 分辨率
    int height;
    int is_primary;           // 是否主显示器
} WgcMonitorInfo;

// ============================================================================
// DLL 导出函数
// ============================================================================

/**
 * 获取可用的监视器数量
 */
WGC_API int wgc_get_monitor_count();

/**
 * 获取监视器信息
 * @param index 监视器索引 (0 ~ count-1)
 * @param info 输出监视器信息
 * @return 成功返回 1，失败返回 0
 */
WGC_API int wgc_get_monitor_info(int index, WgcMonitorInfo* info);

/**
 * 获取所有窗口数量
 */
WGC_API int wgc_get_window_count();

/**
 * 获取窗口信息
 * @param index 窗口索引 (0 ~ count-1)
 * @param info 输出窗口信息
 * @return 成功返回 1，失败返回 0
 */
WGC_API int wgc_get_window_info(int index, WgcWindowInfo* info);

/**
 * 刷新窗口列表（调用前先调用此函数更新窗口列表）
 */
WGC_API void wgc_refresh_windows();

/**
 * 创建捕获会话
 * @param type 捕获类型 (MONITOR 或 WINDOW)
 * @param id 监视器索引 或 窗口句柄 (HWND)
 * @return 捕获句柄，失败返回 NULL
 */
WGC_API HWgcCapture wgc_create_capture(WgcCaptureType type, long long id);

/**
 * 捕获下一帧
 * @param handle 捕获句柄
 * @param frame_info 输出帧信息
 * @return 成功返回 1，暂无新帧返回 0，失败返回 -1
 */
WGC_API int wgc_capture_frame(HWgcCapture handle, WgcFrameInfo* frame_info);

/**
 * 复制帧到 CPU 内存（用于非 GPU Direct 场景）
 * @param handle 捕获句柄
 * @param buffer 输出缓冲区
 * @param buffer_size 缓冲区大小
 * @return 成功返回 1，失败返回 0
 */
WGC_API int wgc_copy_frame_to_cpu(HWgcCapture handle, unsigned char* buffer, int buffer_size);

/**
 * 获取 D3D11 设备（用于 GPU Direct）
 */
WGC_API void* wgc_get_d3d11_device(HWgcCapture handle);

/**
 * 获取 D3D11 上下文
 */
WGC_API void* wgc_get_d3d11_context(HWgcCapture handle);

/**
 * 释放捕获会话
 */
WGC_API void wgc_free_capture(HWgcCapture handle);

#ifdef __cplusplus
}
#endif

// ============================================================================
// C++ 类定义
// ============================================================================

/**
 * WGC 捕获器实现类
 */
class WgcCapturer {
public:
    WgcCapturer();
    ~WgcCapturer();

    // 初始化
    bool Initialize(WgcCaptureType type, long long id);

    // 捕获
    bool Capture(WgcFrameInfo* frame_info);

    // 复制到 CPU
    bool CopyToCPU(unsigned char* buffer, int buffer_size);

    // 获取 D3D11 设备
    ID3D11Device* GetD3D11Device() const { return d3d11_device_.Get(); }
    ID3D11DeviceContext* GetD3D11Context() const { return d3d11_context_.Get(); }

    // 是否已初始化
    bool IsInitialized() const { return initialized_; }

    // 释放资源
    void Release();

private:
    // 创建 D3D11 设备
    bool CreateD3D11Device();

    // 初始化屏幕捕获
    bool InitializeMonitorCapture(int monitor_index);

    // 初始化窗口捕获
    bool InitializeWindowCapture(HWND hwnd);

    // 处理帧
    void ProcessFrame(ID3D11Texture2D* texture);

    // 复制纹理到 staging
    bool CopyToStaging(ID3D11Texture2D* source);

    // D3D11 设备
    ComPtr<ID3D11Device> d3d11_device_;
    ComPtr<ID3D11DeviceContext> d3d11_context_;

    // Staging 纹理（用于 CPU 读取）
    ComPtr<ID3D11Texture2D> staging_texture_;

    // 当前捕获的纹理
    ComPtr<ID3D11Texture2D> captured_texture_;

    // WGC 捕获项 (Windows.Graphics.Capture.Interop)
    IUnknown* capture_item_;
    IUnknown* frame_pool_;
    IUnknown* session_;

    // 状态
    bool initialized_;
    int width_;
    int height_;
    unsigned int frame_id_;

    // 事件句柄
    HANDLE frame_event_;
    HANDLE stop_event_;
};
