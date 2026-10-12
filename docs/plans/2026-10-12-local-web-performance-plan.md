# 本机网页性能执行计划

1. 已完成：恢复既有时间同步，验证真实 WSS challenge 和已安装服务认证/临时访问状态。
2. 执行隔离 NVENC 编码能力测试，构建和临时文件全部在 L 盘。
3. 前端独立任务：先测试档位提交的 profile，再添加连接前档位控件；默认保持现有参数。
4. Windows Agent 独立任务：先测试编码选择与回退，再接入 NVENC；不修改身份或授权边界。
5. 服务任务：先测试 grant fps 到 peer config 的绑定，再修正配置。
6. 运行对应单包、前端与类型检查，独立审查修改；构建可核验的候选程序。
7. 恢复网页/桌面工具入口；真实授权后运行本机正式网页功能和性能测试，记录实际边界。

所有结果保存在 L:\project\mini-remote-desktop\target\self-web-performance-20261012。源码在 codex/local-web-performance-20261012 分支工作树中，主检出中的用户未跟踪文件不动。
