# kiroku

本地视频媒体库整理工具：识别文件 → 获取元数据 → 预览整理计划 → 移动文件、写 NFO 与封面 → 海报墙浏览 → 系统播放器播放。

单进程桌面应用（Tauri 2 + Rust + Svelte），没有服务端；支持 macOS / Windows。

> **声明**：本项目仅用于整理个人本地媒体库，不提供、不存储、不分发任何媒体内容。获取的元数据与图片版权归原网站及权利人所有。如有侵权，请联系删除。

## 功能

- **导入整理**：扫描文件夹 → 获取元数据 → 预览计划（冲突默认跳过）→ 执行；支持移动、复制、硬链接、软链接。
- **媒体库**：海报墙搜索、筛选、收藏；详情编辑与重新获取；追加视频或字幕；库健康检查。
- **历史**：每次整理都记录操作日志，可撤销。
- **元数据源**：多个可选来源，支持按来源配置代理、Cookie 与字段优先级。
- **其他**：命名模板、封面裁剪、字幕随迁、AI 翻译（自备 Anthropic API Key）、Plex / Jellyfin / Emby 刷新。

## 安装

在仓库 [Releases](../../releases) 页面下载对应平台的安装包：

| 平台 | 文件 |
|---|---|
| macOS（Apple Silicon） | `kiroku_<版本>_aarch64.dmg` |
| macOS（Intel） | `kiroku_<版本>_x64.dmg` |
| Windows | `kiroku_<版本>_x64-setup.exe` 或 `.msi` |

安装包未签名。macOS 首次打开如提示「无法验证开发者」，在 Finder 中右键 → 打开；Windows 如出现 SmartScreen 提示，选择「仍要运行」。

## 开发

```bash
cargo test --workspace                     # 核心 + CLI 测试
cd apps/desktop && pnpm install
pnpm dev                                   # 仅前端：浏览器里用 mock 后端预览
pnpm tauri dev                             # 桌面客户端
pnpm tauri build                           # 本地出安装包
pnpm test:e2e                              # 界面回归（首次需 pnpm exec playwright install chromium）
```

CLI（逻辑与客户端共用，默认只预览，加 `--apply` 才执行）：

```bash
cargo run -p kiroku-cli -- parse <目录>
cargo run -p kiroku-cli -- scrape <目录> --out <库> [--apply]
cargo run -p kiroku-cli -- rollback <库>/.kiroku/journal-*.json
cargo run -p kiroku-cli -- index <库> [关键词]
```

## 发布

1. 修改 `apps/desktop/src-tauri/tauri.conf.json` 与根 `Cargo.toml` 中的 `version`，提交。
2. 打标签并推送：`git tag v0.1.0 && git push origin v0.1.0`。
3. GitHub Actions 的 Release 流程会构建 macOS（arm64 / x64）与 Windows 安装包，全部平台成功后自动发布到 Releases 页面供下载；标签与 `tauri.conf.json` 版本不一致时流程会直接失败。

也可以在 Actions 页面手动运行 Release 流程，标签按 `tauri.conf.json` 的版本号生成。

## 结构

```
crates/kiroku-core   领域核心：文件名解析、元数据源、聚合、整理、NFO、索引、操作日志
crates/kiroku-cli    命令行
apps/desktop         Tauri 2 客户端：src-tauri（Rust 命令）+ src（Svelte 5 界面与 mock 后端）
docs/                架构决策（ADR）、使用说明、路线
```

磁盘是唯一真相（视频同名 NFO + `kiroku.json`），SQLite 只是可重建的索引。开发约束见 [AGENTS.md](AGENTS.md)。

## 许可

[GPL-3.0-or-later](LICENSE)。部分文件名解析逻辑移植自 [amane](https://github.com/sqzw-x/amane)（GPL-3.0），保持同许可证。
