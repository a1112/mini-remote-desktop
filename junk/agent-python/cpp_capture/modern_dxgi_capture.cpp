/**
 * Desktop Duplication 1.5 - 支持多并发捕获器.
 *
 * 使用 IDXGIOutput5::DuplicateOutput1 替代 IDXGIOutput1::DuplicateOutput
 * 这样可以与 Windows Game Bar 等应用共存.
 */

#ifdef D3D12_HYBRID_CAPTURE_EXPORTS
#undef D3D12_HYBRID_CAPTURE_EXPORTS
#endif
#define D3D12_HYBRID_CAPTURE_EXPORTS

#include <windows.h>
#include <d3d11.h>
#include <dxgi1_5.h>
#include <wrl/client.h>
#include <iostream>
#include <vector>

using Microsoft::WRL::ComPtr;

// 新的 DXGI 1.5 捕获器
class ModernDXGICapturer {
public:
    ModernDXGICapturer() : initialized_(false) {}

    bool Initialize(int monitor_index = 0) {
        // 创建 D3D11 设备
        D3D_FEATURE_LEVEL feature_levels[] = {
            D3D_FEATURE_LEVEL_11_1,
            D3D_FEATURE_LEVEL_11_0,
        };

        D3D_FEATURE_LEVEL selected_level;
        HRESULT hr = D3D11CreateDevice(
            nullptr,
            D3D_DRIVER_TYPE_HARDWARE,
            nullptr,
            0,
            feature_levels,
            ARRAYSIZE(feature_levels),
            D3D11_SDK_VERSION,
            &d3d11_device_,
            &selected_level,
            &d3d11_context_
        );

        if (FAILED(hr)) {
            std::cerr << "[ModernDXGI] D3D11CreateDevice failed: " << std::hex << hr << std::endl;
            return false;
        }

        // 获取 DXGI 设备
        ComPtr<IDXGIDevice> dxgi_device;
        hr = d3d11_device_.As(&dxgi_device);
        if (FAILED(hr)) {
            std::cerr << "[ModernDXGI] Get DXGI device failed" << std::endl;
            return false;
        }

        // 获取适配器
        ComPtr<IDXGIAdapter> adapter;
        hr = dxgi_device->GetAdapter(&adapter);
        if (FAILED(hr)) {
            std::cerr << "[ModernDXGI] Get adapter failed" << std::endl;
            return false;
        }

        // 尝试获取 DXGI 1.5 适配器 (用于新 API)
        ComPtr<IDXGIAdapter1> adapter1;
        hr = adapter.As(&adapter1);
        if (FAILED(hr)) {
            std::cout << "[ModernDXGI] DXGI 1.5 not available, trying legacy..." << std::endl;
            return TryLegacyDuplication(monitor_index);
        }

        // 尝试获取 DXGI 1.5 输出
        ComPtr<IDXGIOutput> output;
        hr = adapter1->EnumOutputs(monitor_index, &output);
        if (FAILED(hr)) {
            std::cerr << "[ModernDXGI] EnumOutputs failed" << std::endl;
            return false;
        }

        // 尝试 DXGI 1.5 输出
        ComPtr<IDXGIOutput1> output1;
        hr = output.As(&output1);
        if (FAILED(hr)) {
            return TryLegacyDuplication(monitor_index);
        }

        // 尝试 DXGI 1.6 输出 (支持 DuplicateOutput1)
        ComPtr<IDXGIOutput6> output6;
        hr = output.As(&output6);
        if (SUCCEEDED(hr)) {
            std::cout << "[ModernDXGI] DXGI 1.6 available" << std::endl;
            return TryModernDuplication1(output6.Get());
        }

        ComPtr<IDXGIOutput5> output5;
        hr = output.As(&output5);
        if (SUCCEEDED(hr)) {
            std::cout << "[ModernDXGI] DXGI 1.5 available, using DuplicateOutput1" << std::endl;
            return TryModernDuplication1(output5.Get());
        }

        return TryLegacyDuplication(monitor_index);
    }

    bool TryModernDuplication1(IDXGIOutput5* output) {
        // 使用新的 DuplicateOutput1 API
        // 支持多个并发捕获器
        HRESULT hr = output->DuplicateOutput1(
            d3d11_device_.Get(),
            0,  // flags
            nullptr,  // supported_formats
            &duplication_
        );

        if (SUCCEEDED(hr)) {
            std::cout << "[ModernDXGI] DuplicateOutput1 succeeded!" << std::endl;
            initialized_ = true;
            return true;
        }

        if (hr == DXGI_ERROR_ACCESS_DENIED) {
            std::cerr << "[ModernDXGI] Access denied (even with DuplicateOutput1)" << std::endl;
            std::cerr << "[ModernDXGI] Possible reasons:" << std::endl;
            std::cerr << "[ModernDXGI]  - Desktop is locked" << std::endl;
            std::cerr << "[ModernDXGI]  - On login screen" << std::endl;
            std::cerr << "[ModernDXGI]  - Another app has exclusive access" << std::endl;
        } else {
            std::cerr << "[ModernDXGI] DuplicateOutput1 failed: " << std::hex << hr << std::endl;
        }

        return false;
    }

    bool TryLegacyDuplication(int monitor_index) {
        std::cout << "[ModernDXGI] Trying legacy DuplicateOutput..." << std::endl;

        ComPtr<IDXGIDevice> dxgi_device;
        if (FAILED(d3d11_device_.As(&dxgi_device))) {
            return false;
        }

        ComPtr<IDXGIAdapter> adapter;
        if (FAILED(dxgi_device->GetAdapter(&adapter))) {
            return false;
        }

        ComPtr<IDXGIOutput> output;
        if (FAILED(adapter->EnumOutputs(monitor_index, &output))) {
            return false;
        }

        ComPtr<IDXGIOutput1> output1;
        if (FAILED(output.As(&output1))) {
            return false;
        }

        HRESULT hr = output1->DuplicateOutput(d3d11_device_.Get(), &duplication_);
        if (SUCCEEDED(hr)) {
            std::cout << "[ModernDXGI] Legacy DuplicateOutput succeeded!" << std::endl;
            initialized_ = true;
            return true;
        }

        std::cerr << "[ModernDXGI] Legacy DuplicateOutput also failed: " << std::hex << hr << std::endl;
        return false;
    }

    bool IsInitialized() const { return initialized_; }

private:
    ComPtr<ID3D11Device> d3d11_device_;
    ComPtr<ID3D11DeviceContext> d3d11_context_;
    ComPtr<IDXGIOutputDuplication> duplication_;
    bool initialized_;
};

// 导出测试函数
extern "C" {

__declspec(dllexport) int test_modern_dxgi() {
    ModernDXGICapturer capturer;
    if (capturer.Initialize()) {
        return 1;
    }
    return 0;
}

}
