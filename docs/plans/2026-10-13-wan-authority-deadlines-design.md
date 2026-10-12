# WAN 信令与会话授权期限

本次工作遵循用户已有的自动修复和实测授权。目标是已配对浏览器在密码未改变时自动确认，并完成真实画面和键鼠测试。

## 已验证的问题

本机两次网页会话在目标批准及获取 relay access 后被撤销，随后网页 bootstrap 返回 404、close 返回 409。现场 API 的 grant 和 policy TTL 均为 600 秒。纯内存 coordinator 重现证明：60 秒 intent 搭配 50 秒 grant 成功，搭配 600 秒 grant 则在签发 grant 后因 PolicyMismatch 失败并撤销。

当前代码把信令投递有效期用于整个会话。网页 intent 是 60 秒，原生信令是 30 秒，backend grant 可能更长；controller 还把新 grant 的投递截止与旧 intent 截止比较。这些值承担不同责任。

## 选择

把后端所有会话缩为 30 秒会破坏正常远控和性能采样。增加协议字段及升级 wire 暂无必要：现有签名 grant 已覆盖 policy deadline，已认证 backend 提供 exact grant deadline。采用这两个现有凭证的组合证明；未经证明的 snapshot 或 GrantBinding 不能延长期限。

保留 immutable setup deadline，所有确认及授权前等待必须在它之前完成。仅当请求、身份公钥、intent/grant commitment、revision、scopes、profile、route 和期限通过既有严格校验后，内部组合边界生成私有 active-authority proof。它一次性绑定运行期限 min(backend grant, signed policy)，两期限必须有效且不超过现有后端允许的一小时上界。重复消息只保留相同授权，不能续期。

运行中任务使用已绑定期限，继续精确核对 backend。期限、权限、密码或策略变化触发清理，不能更新为更晚截止。信令自身仍严格验证签名、新鲜度、受众和 replay。

Windows registry 只允许私有的已证明 WAN 安装入口在当前本机确认尚有效时交换后端 lease。最终 registry 锁内重新读取可靠时间，避免跨 await 后使用旧时间。LAN 和普通 grant 安装保持既有限制。Agent 的独立五分钟权限上限保持，因此本轮不能声称原生资源可持续十分钟。

## 验证和完成条件

覆盖短消息和长授权、晚到确认、锁等待跨截止、未证明 grant、过期或超上界期限、错误身份/策略/commitment、重复及尝试延长授权，以及运行超过原信令截止后的真实截止清理。独立审阅后构建并正常升级本机 service，重新实测配对、动态视频和远程记事本输入；通过后才采样性能。组件通过不代表端到端成功。
