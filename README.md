# kiroku

本地媒体刮削与整理工具。解析文件名中的编号，从多个来源获取元数据，预览整理计划，再生成 NFO、封面和规范文件名；整理完成后通过海报墙浏览、编辑与播放。

基于 **Rust + Tauri 2 + Svelte**，采用单进程桌面架构，无需部署服务器或 Docker。个人自用项目，持续迭代中。

[下载安装包](https://github.com/Leo0426/kiroku/releases/latest) · [更新记录](docs/releases/) · [路线图](docs/roadmap.md)

## 安装

在 [GitHub Releases](https://github.com/Leo0426/kiroku/releases) 选择对应平台的安装包：

| 平台 | 安装包 |
| --- | --- |
| macOS Apple Silicon | `aarch64.dmg` |
| macOS Intel | `x64.dmg` |
| Windows x64 | `.exe` 或 `.msi` |

macOS 打开 DMG 后将 kiroku 拖入“应用程序”。安装包尚未配置 Developer ID 公证或 Windows 代码签名，系统可能显示开发者验证提示。Windows 的部分文件系统场景仍在适配，具体状态见路线图。

## 第一次使用

1. **配置媒体库**：在设置中选择整理目标目录，按需启用数据源，设置路由、代理和访问凭据。
2. **选择待整理目录**：扫描视频并识别编号、分片、字幕及版本标记；识别有误时可先修正编号。
3. **获取元数据**：按内容类型选择来源，按字段优先级聚合结果；可配置名称规则和 AI 汉化。
4. **预览并确认整理**：核对目标路径、冲突和文件处理方式，再执行。支持移动、复制、硬链接、软链接。
5. **浏览与修订**：在媒体库搜索、筛选、播放，或打开详情编辑信息。需要撤销时前往“历史”。

整理会处理真实文件，请先用少量文件确认命名模板和处理方式符合预期。界面确认前不会执行整理；CLI 默认只预览，传入 `--apply` 才落盘。

## 主要功能

### 导入与文件整理

- 编号解析、分片识别、字幕配对，以及多来源并行获取和字段聚合。
- 自定义目录与文件名模板；写入视频同名 NFO、`kiroku.json`、poster / fanart 和可选剧照。
- 已有影片追加视频或字幕，同编号不同版本可并存；普通导入中的新版本需要手动选择。
- 支持 `-C`、`-U`、`-UC`、`-UMR`、`-LTX` 等标记。`-LTX` 归为破解版本，不自动推断字幕；默认整理命名为 `-U`，带字幕时为 `-UC`。
- 任务进度、运行日志、停止任务与操作历史；跨盘移动先复制校验，再删除源文件。

### 媒体库与人工编辑

- 海报墙 / 列表、搜索筛选、收藏、保存视图、连续浏览、随机挑选与 CSV 导出。
- 详情侧栏支持拖动左边缘调整宽度、记住宽度、双击复位，以及窄窗口自适应。
- 人工编辑标题、简介、演员、类别等元数据，先预览差异再保存；普通字段保存后锁定，支持历史撤销。
- **手动补版本标记**：在“编辑元数据 → 版本标记”中补充字幕、破解（含 LTX）、流出或清晰度。普通分类标签可在“类别”中逐行追加。
- 保留原图、裁剪海报；修改版本标记时，从可用原图重绘海报。缺少原图或无法确认裁剪范围时提示并保留原海报，不在旧角标上重复叠图。角标需在设置中启用，背景图不会随此操作重绘。
- 来源重选、选择性更新、名称别名规范化、文本语言版本与 NFO 语言导出。

### 外部服务与维护

- AI 汉化支持 Anthropic Messages 和 OpenAI Chat Completions 兼容协议，使用自己的服务端点与 API Key。
- 支持 Plex、Jellyfin / Emby 局部刷新与路径映射；播放交给外部播放器。Plex 主要依赖文件命名和图片，不读取 NFO。
- 来源连通性检测、媒体库健康检查、补缺图、重建索引，以及带预览的历史和生成数据清理。

## 数据与撤销

影片目录中的 **视频、NFO 和 `kiroku.json` 是数据来源**；`<媒体库>/.kiroku/library.db` 只是可重建的索引缓存。CLI 与桌面端共用系统配置目录下的 `kiroku/settings.json`。

整理和人工修订会保存操作日志；取消或部分失败后可在历史中核对已完成操作。撤销会检查文件身份与内容，遇到外部修改时保留文件并报告冲突，不能替代独立的数据备份。原目录清理使用系统废纸篓；保留源文件的处理模式不会清理源目录。

元数据获取、图片下载、AI 汉化和媒体服务器通知需要访问你配置的外部服务。凭据只应保存在本机配置中，不要提交到仓库。

## 本地开发

需要 Rust stable、Node.js 22、pnpm 10，以及 Tauri 对应平台的原生编译工具链。

```sh
# 安装前端依赖
cd apps/desktop
pnpm install --frozen-lockfile

# 启动桌面应用
pnpm tauri dev

# 仅预览前端：自动使用模拟后端，可走完整流程
pnpm dev

# 构建当前平台安装包
pnpm tauri build
```

仓库结构：

```text
crates/kiroku-core/       领域核心、解析、刮削、整理与索引
crates/kiroku-cli/        命令行入口
apps/desktop/            Svelte 界面
apps/desktop/src-tauri/  Tauri 命令与桌面集成
docs/                    架构决策、路线图和发布说明
```

在仓库根目录运行检查：

```sh
cargo test
cargo test -p kiroku-core --test golden_file_info
cargo check -p kiroku-desktop
cd apps/desktop
pnpm check
pnpm build
pnpm exec playwright install chromium
pnpm exec playwright test
```

CLI 示例（在仓库根目录运行）：

```sh
cargo run -p kiroku-cli -- parse /path/to/inbox
cargo run -p kiroku-cli -- scrape /path/to/inbox --out /path/to/library
# 核对预览后添加 --apply 执行
cargo run -p kiroku-cli -- health /path/to/library
```

开发约束见 [AGENTS.md](AGENTS.md)，领域术语见 [CONTEXT.md](CONTEXT.md)，架构决策见 [docs/adr](docs/adr/)。测试和模拟数据使用中性示例，不提交真实抓取页面或私人媒体资料。

## 许可与致谢

采用 [GPL-3.0-or-later](LICENSE)。部分文件名解析逻辑移植自 [amane](https://github.com/sqzw-x/amane)（GPL-3.0），保持同许可证。
