# DXGI C++ DLL 编译指南

## 文件清单

```
cpp_capture/
├── dxgi_capture.h      # 头文件
├── dxgi_capture.cpp    # 实现文件
├── CMakeLists.txt      # CMake 配置
├── build.bat           # 构建脚本
└── README.md           # 文档
```

## Visual Studio 编译步骤

### 方法 1: 使用开发者命令行 (推荐)

1. 打开 **x64 Native Tools Command Prompt for VS 2022**
   - 开始菜单 → Visual Studio 2022

2. 进入目录并编译:
```bash
cd J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture

# 直接编译
cl.exe /LD /MD /O2 /EHsc dxgi_capture.cpp /link d3d11.lib dxgi.lib

# 或使用脚本
build.bat
```

### 方法 2: 在 Visual Studio IDE 中

1. 打开 Visual Studio 2022
2. 创建新项目 → **动态链接库 (DLL)** → x64
3. 添加源文件到项目
4. 项目属性配置:
   ```
   配置类型: 动态库 (.dll)
   字符集: 使用 Unicode
   C++ 语言标准: ISO C++17 /std:c++17
   ```
5. 链接器 → 输入 → 附加依赖项:
   ```
   d3d11.lib
   dxgi.lib
   ```
6. 生成 → 生成解决方案

### 方法 3: 使用 CMake

```bash
cd cpp_capture
cmake -B build -A x64
cmake --build build --config Release
```

## 编译完成后

1. 将生成的 `dxgi_capture.dll` 复制到项目根目录:
   ```
   J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\dxgi_capture.dll
   ```

2. 测试:
   ```bash
   python src/capture/dxgi_cpp.py
   ```

## 预期结果

```
======================================================================
DXGI C++ DLL 捕获测试
======================================================================
✅ DXGI 捕获器初始化成功
   分辨率: 2560x1440

性能测试 (5秒)...

结果:
  捕获帧数: 600+
  FPS: 120+
  平均延迟: <5 ms

对比:
  DXGI C++:   120+ FPS  🚀
  d3dshot:    ~60 FPS     🚀
  MSS:        ~30 FPS     ⚡
```

## 故障排除

### 编译错误

**错误: 无法打开包括文件: 'd3d11.h'**
```
解决方案: 安装 Windows 10 SDK
Visual Studio Installer → 修改 → Windows 10 SDK
```

**错误: LNK2019: 无法解析的外部命令**
```
解决方案: 添加链接库
项目属性 → 链接器 → 输入 → 附加依赖项
添加: d3d11.lib dxgi.lib
```

### 运行时错误

**错误: Failed to load DXGI DLL**
```
解决方案:
1. 确认 DLL 已编译
2. 检查 DLL 是 x64 版本 (匹配 Python)
3. 将 DLL 放在正确位置
```

**错误: Desktop Duplication access denied**
```
解决方案: 以管理员身份运行
或确保没有其他应用使用 DXGI (如 OBS、录屏软件)
```

## 集成到现有代码

```python
# 在现有代码中使用
from capture.dxgi_cpp import create_dxgi_capture

# 创建捕获器
capture = create_dxgi_capture()
if capture:
    while True:
        frame = capture.capture()  # numpy array
        # 使用 frame...
```
