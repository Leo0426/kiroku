# macOS 原生退出验收

在已登录的 macOS 图形会话、仓库根目录运行：

```sh
python3 tools/check_native_shutdown.py
# 只复验指定场景
python3 tools/check_native_shutdown.py idle fast
```

工具编译并逐次启动 `native_shutdown_smoke` example，复用生产的 AppState、退出事件处理和 AppKit 桥接，使用真实 Wry 事件循环和临时窗口。Tauri 的 test helper 只提供空 context/assets，不使用 MockRuntime。任务完全在内存中合成，不加载用户设置、访问媒体或操作废纸篓；`cargo test` 不启动这个 GUI example。

| 模式 | 验收内容 |
|---|---|
| close | 忙碌时重复关闭主窗口 |
| native | 忙碌时重复调用 AppKit `terminate:` |
| combined | 关闭窗口后再请求原生退出 |
| exit | 忙碌时请求带退出码 23 的 Tauri 退出 |
| idle | 空闲时立即原生退出 |
| fast | 收到退出取消信号后立即完成任务 |

四个慢任务场景在取消信号后继续模拟提交 6 秒，确认进程与窗口仍在、新任务被拒绝，然后释放 JobGuard。脚本检查任务完成先于退出、退出时无活动任务、重复请求与退出码；每个子进程最多运行 20 秒，超时仅终止本次合成探针并报错。

原生退出使用 Cmd+Q、应用菜单及 Dock Quit 所用的 AppKit selector。探针在主线程安排延时 selector，让实际调用发生在 Tao 回调之外；直接在 Tao UserEvent 回调里同步调用会重入 Tao 自己的回调锁。这里覆盖的是原生入口及生命周期，不替代真实媒体写入中的手动验收，也未逐一操作菜单和 Dock。

## 退出约定与运行时升级

首次关闭请求原子地停止接收新任务并取消当前令牌；不可中断的提交继续收尾，匹配的 JobGuard 释放后才退出，没有固定 5 秒截止。等待时主窗口保留并显示收尾标题。重复请求不重复建立等待任务。

当前锁定的 Tauri/Tao 原生 `terminate:` 路径不经过 `RunEvent::ExitRequested`。桥接保留 Tao delegate，只添加当前缺少的 `applicationShouldTerminate:` 决策；若升级后发现本地或继承的实现，启动明确报错，需重新检查兼容性。相关上游讨论见 [Tauri #12978](https://github.com/tauri-apps/tauri/issues/12978)，Apple 的协议见 [applicationShouldTerminate](https://developer.apple.com/documentation/appkit/nsapplicationdelegate/applicationshouldterminate(_:))。

忙碌时桥接返回 `NSTerminateCancel`，任务结束后再从 Tauri 发起退出；空闲返回 `NSTerminateNow`。因此系统注销、重启或关机触发忙碌退出时，系统请求会被取消，任务结束只退出本应用，需要用户重新发起系统操作。这不是对系统请求的透明延迟。强制结束进程、崩溃和断电不在本保护范围内，仍依赖持久日志尽力恢复。
