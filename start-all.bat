@echo off
echo ========================================
echo Mini Remote Desktop - 一键启动
echo ========================================
echo.

REM 检查 Node.js
node --version >nul 2>&1
if errorlevel 1 (
    echo [错误] 请先安装 Node.js
    pause
    exit /b 1
)

echo [1/2] 启动信令服务器...
start cmd /k "cd server && npm install && node index.js"

timeout /t 2 /nobreak >nul

echo [2/2] 启动被控端 Agent...
start cmd /k "cd agent && npm install && npm start"

echo.
echo ========================================
echo 服务已启动！
echo ========================================
echo.
echo 访问地址: 打开 web/index.html
echo 或者: http://localhost:5500 (使用 Live Server)
echo.
echo 信令服务器: ws://localhost:9527
echo.
echo 按任意键关闭此窗口...
pause >nul
