cmake_minimum_required(VERSION 3.15)
project(GPU_Direct_Capture)

# 设置 C++ 标准
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# 输出目录
set(CMAKE_RUNTIME_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/bin)
set(CMAKE_LIBRARY_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/lib)
set(CMAKE_ARCHIVE_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/lib)

# =============================================================================
# 依赖路径配置
# =============================================================================
# NVENC SDK 路径 - 根据实际安装位置调整
set(NVENC_SDK_PATH "J:/ProjectTest/远程探查/mini-remote-desktop/tools/Video_Codec_Interface_13.0.37/Interface" CACHE PATH "NVENC SDK Path")

# CUDA 路径 - 根据实际安装位置调整
set(CUDA_PATH "C:/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v12.6" CACHE PATH "CUDA Path")

# DirectX SDK 包含在 Windows SDK 中
# =============================================================================
# 公共定义
# =============================================================================
add_definitions(-DDXGI_CAPTURE_EXPORTS)
add_definitions(-DNVENC_ENCODER_EXPORTS)
add_definitions(-DD3D12_HYBRID_CAPTURE_EXPORTS)

# =============================================================================
# 1. d3d12_hybrid_capture.dll - D3D11/D3D12 混合捕获
# =============================================================================
set(HYBRID_SOURCES
    d3d12_hybrid_capture.cpp
)

set(HYBRID_HEADERS
    d3d12_hybrid_capture.h
)

add_library(d3d12_hybrid_capture SHARED ${HYBRID_SOURCES} ${HYBRID_HEADERS})

target_include_directories(d3d12_hybrid_capture PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_link_libraries(d3d12_hybrid_capture
    PRIVATE
    d3d11.lib
    d3d12.lib
    dxgi.lib
    dxguid.lib
)

# =============================================================================
# 2. nvenc_full.dll - NVENC 编码器 (D3D11-CUDA 互操作)
# =============================================================================
set(NVENC_SOURCES
    nvenc_full.cpp
)

set(NVENC_HEADERS
    nvenc_full.h
)

add_library(nvenc_full SHARED ${NVENC_SOURCES} ${NVENC_HEADERS})

target_include_directories(nvenc_full PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${NVENC_SDK_PATH}
    ${CUDA_PATH}/include
)

target_link_libraries(nvenc_full
    PRIVATE
    d3d11.lib
    ${CUDA_PATH}/lib/x64/cuda.lib
    ${CUDA_PATH}/lib/x64/nvencodeapi.lib
)

# =============================================================================
# 3. dxgi_capture.dll - 基础 DXGI 捕获 (保留兼容)
# =============================================================================
set(DXGI_SOURCES
    dxgi_capture.cpp
)

set(DXGI_HEADERS
    dxgi_capture.h
)

add_library(dxgi_capture SHARED ${DXGI_SOURCES} ${DXGI_HEADERS})

target_link_libraries(dxgi_capture
    PRIVATE
    d3d11.lib
    dxgi.lib
)

# =============================================================================
# Windows 特定设置
# =============================================================================
if(WIN32)
    # 设置运行时库为静态链接
    set_property(TARGET d3d12_hybrid_capture PROPERTY
        MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
    set_property(TARGET nvenc_full PROPERTY
        MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
    set_property(TARGET dxgi_capture PROPERTY
        MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
endif()

# =============================================================================
# 编译选项
# =============================================================================
if(MSVC)
    target_compile_options(d3d12_hybrid_capture PRIVATE
        /W4
        /wd4100  # 未使用的形参
        /wd4201  # 匿名结构体
    )

    target_compile_options(nvenc_full PRIVATE
        /W4
        /wd4100
        /wd4201
    )

    target_compile_options(dxgi_capture PRIVATE
        /W4
        /wd4100
        /wd4201
    )
endif()

# =============================================================================
# 安装/复制规则
# =============================================================================
# 复制所有 DLL 到 Python 项目目录
function(copy_dll_to_python target_name)
    add_custom_command(TARGET ${target_name} POST_BUILD
        COMMAND ${CMAKE_COMMAND} -E copy
            $<TARGET_FILE:${target_name}>
            ${CMAKE_SOURCE_DIR}/../$<TARGET_FILE_NAME:${target_name}>
        COMMENT "Copying ${target_name}.dll to Python project"
    )
endfunction()

copy_dll_to_python(d3d12_hybrid_capture)
copy_dll_to_python(nvenc_full)
copy_dll_to_python(dxgi_capture)

# =============================================================================
# 安装目标
# =============================================================================
install(TARGETS d3d12_hybrid_capture nvenc_full dxgi_capture
    RUNTIME DESTINATION bin
    LIBRARY DESTINATION lib
)
