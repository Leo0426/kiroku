# macOS 本地打包与安装

发布前完成前端检查、构建及相关 Rust/浏览器回归。版本号沿用 Cargo workspace 与 Tauri 配置；本地构建以提交 SHA 区分。

生产资产还应通过原生 WebKit 验收，分别覆盖首次引导和已有空库：

```sh
cd apps/desktop
pnpm build
cd ../..
cargo run -p kiroku-desktop --release --example release_ui_smoke --features tauri/custom-protocol
cargo run -p kiroku-desktop --release --example release_ui_smoke --features tauri/custom-protocol -- --library
```

该检查使用生产 CSP、嵌入资产、真实 command 注册与 IPC；设置、库和 WebView 存储均隔离，退出后清理临时目录。渲染错误、错误页面或 10 秒未就绪均失败。它不替代安装应用在实际用户环境中的启动检查。

打包命令为 `cd apps/desktop && pnpm exec tauri build --bundles app,dmg --ci`。如果本机不允许 Finder 自动化，DMG 的外观布置脚本可能返回 `-1743`；可仅构建 app，使用 `hdiutil create` 把已签名 app 与 `/Applications` 链接封装为普通 DMG，无需 Finder 自动化。

替换前备份旧 app 和设置；本机使用 ad-hoc 签名时，以 `codesign --verify --deep --strict` 验证。安装前核对 DMG 校验和及其中 app 的签名和可执行文件摘要，先复制到暂存目录再替换 `/Applications/kiroku.app`。保留备份，失败时退出新应用并恢复旧 app；SQLite v5 仅增加可重建缓存索引。

安装后确认应用能重复启动、主导航可见、媒体库概览能通过原生后端读取。初次窗口空白不能判为交付通过；应完整退出后重试、运行隔离验收并检查日志，仍无法启动则恢复备份。记录提交、产物 SHA-256、备份位置、兼容边界和实际验收结果至忽略的 `.scratch/releases/`，不提交用户设置或媒体数据。
