cmake_minimum_required(VERSION 3.15)
project(DXGI_Capture)

# 设置 C++ 标准
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# 输出目录
set(CMAKE_RUNTIME_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/bin)
set(CMAKE_LIBRARY_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/lib)
set(CMAKE_ARCHIVE_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/lib)

# DLL 定义
add_definitions(-DDXGI_CAPTURE_EXPORTS)

# 源文件
set(SOURCES
    dxgi_capture.cpp
)

# 头文件
set(HEADERS
    dxgi_capture.h
)

# 创建 DLL
add_library(dxgi_capture SHARED ${SOURCES} ${HEADERS})

# 链接库
target_link_libraries(dxgi_capture
    PRIVATE
    d3d11.lib
    dxgi.lib
)

# Windows 特定设置
if(WIN32)
    # 设置运行时库为静态链接 (避免依赖 MSVC 运行时 DLL)
    set_property(TARGET dxgi_capture PROPERTY
        MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
endif()

# 编译选项
if(MSVC)
    target_compile_options(dxgi_capture PRIVATE
        /W4
        /wd4100  # 未使用的形参
        /wd4201  # 匿名结构体
    )
endif()

# 复制 DLL 到 Python 项目
add_custom_command(TARGET dxgi_capture POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy
        $<TARGET_FILE:dxgi_capture>
        ${CMAKE_SOURCE_DIR}/../$<TARGET_FILE_NAME:dxgi_capture>
    COMMENT "Copying DLL to Python project"
)
