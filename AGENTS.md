# kiroku

本地媒体刮削与整理工具:编号解析 → 多源爬取 → 字段聚合 → 整理落盘(NFO / 封面 / 命名模板)。
单进程桌面应用,无服务化外壳(无 HTTP server / WebSocket / 监督进程 / Docker)。

## 技术栈与结构
- Rust workspace:`crates/kiroku-core`(领域核心)、`crates/kiroku-cli`(命令行)、`apps/desktop`(Tauri 2 + Svelte 客户端,`src-tauri` 为 Rust 端)
- Tauri 2 桌面 GUI,前端通过 command 进程内调用 core;决策与反爬 PoC 结论见 `docs/adr/0001-single-process-tauri-rust.md`
- 许可证 GPL-3.0-or-later(参考 amane 源码移植须保持同许可证)

## 常用命令
- `cargo test`
- `cargo run -p kiroku-cli -- parse <dir>`
- 桌面客户端:`cd apps/desktop && pnpm install && pnpm tauri dev`;仅预览前端 `pnpm dev`(浏览器里自动用 `src/lib/mock/engine.ts` 模拟后端,可走完整闭环;mock 与 `src-tauri` 命令必须同步更新)
- 前端检查:`pnpm check && pnpm build`;桌面端编译检查:`cargo check -p kiroku-desktop`
- 设置文件 `<系统配置目录>/kiroku/settings.json`,CLI 与客户端共用(`kiroku_core::settings`)

## 架构要点
- 流程集中在 `kiroku_core::pipeline`(scan → scrape_all → build_plans → apply),CLI 与 Tauri 命令共用;核心不做 UI、不依赖 Tauri。
- 磁盘是唯一真相:影片目录里的视频同名 NFO(兼容旧 `movie.nfo`) + `kiroku.json` + 视频;新海报/背景为 `<影片名>-poster.jpg` / `<影片名>-fanart.jpg`,主体名由 sidecar 的 `asset_stem` 固定;`library.rs` 的 SQLite 索引(`<库>/.kiroku/library.db`)只是缓存,**唯一写入口是 `Library::reindex_dir`**(整理、回滚、重建都走它)。
- 整理日志 `<库>/.kiroku/journal-<ts>.json`:`apply` 每完成一项就落盘,取消/失败也有日志;回滚尽力而为并清理空目录。
- Tauri 事件(`scan:items` `scrape:event` `apply:event` `job:eta` `library:*`)都带 `jobId`;同一时间只允许一个任务(`AppState::begin_job`)。
- 运行日志用 `kiroku_core::logging` 按 `Settings.log` 分发；核心 `log` target 为 `kiroku::<模块>`，App 事件为 `log:line`。
- 前端 `lib/api.ts` 是后端门面(Tauri invoke / mock);新增后端能力要同时改 `types.ts`、`api.ts`、`mock/engine.ts`。

## 开发约束
- 编号解析保持纯函数,无 I/O;`code.rs` 与 amane 的 `file_info.py` 逐条对应,改动前后都要跑 `cargo test -p kiroku-core --test golden_file_info`(golden 由 `tools/gen_amane_golden.py` 生成)
- 数据源按**内容类型路由**(`Settings.routes`,编号解析得出类型);字段取值语义对齐 amane:标量取链上第一个非空,URL 类沿链拼接去重,屏蔽优先于提前
- 站点解析函数保持纯函数并用合成 fixture 回归(`tests/fixtures/<站>/`:只保留解析器用到的结构,内容一律用中性占位,不提交真实抓取页面;真实页面只存本地 `.private/`);请求头/Cookie 在 `Source::fetch`,图片同源 Referer 由 `Http` 统一补
- 编号口径跟随 amane:`FC2-PPV-n` → `FC2-n`,不去前导零(站点检索时用 `Code::search_key` 归一)
- 文件操作默认 dry-run;执行产出操作日志可回滚;跨盘先复制校验再删源
- 整理后清理原文件夹只用 `pipeline::trash_to_system`(移到废纸篓,不永久删除),且只限待整理根目录之内;测试必须注入假的 `TrashFn`,不能碰真实废纸篓
- AI 汉化(`translate.rs`)按所选 provider 走 OpenAI Chat Completions / Anthropic Messages 原始 HTTP;`translate/config.rs` 管迁移与校验,`translate/protocol.rs` 管请求/响应。Anthropic 官方预设保留 `claude-opus-5-5` + effort `low` + `fallbacks: "default"`;自定义服务可选提示词 JSON、默认思考强度并关闭 fallback。读内容前检查 `stop_reason` / `finish_reason` 与拒绝字段;失败不跨 provider 重试。旧单组配置仅在读取时迁移,缓存身份包含协议、端点与生成选项。
- Jellyfin/Emby(`media_server.rs`)按路径 `POST /Library/Media/Updated` 通知,可配本机→服务器路径前缀映射;Key 只放请求头
- Plex(`plex.rs`)不读 NFO:靠文件名(`PlanOpts::plex_naming`)和 poster/fanart;整理/撤销后按路径匹配分区做局部刷新
- 磁盘 I/O 不阻塞 UI 线程
- 每个数据源独立模块 + fixture 回归测试;域名进配置
- 范围:选目录 → 刮削 → 整理 → 海报墙 → 调用外部播放器,外加媒体服务器联动(Plex,Jellyfin/Emby 见 ADR-0010)与 AI 汉化;文件监控、超分、播放反代不做(ADR-0001)
- 路线与待办见 `docs/roadmap.md`;站点/编号/字段/NFO 规则参考本地文档 `docs/amane-archaeology.md`(不入库)
- 公开仓库内的文档、fixture、测试与 mock 数据不得出现成人内容描述(真实片名、简介、演员名、类别标签、封面地址等);需要时用中性占位

## Agent skills
### Issue tracker
issues 与 PRD 以本地 markdown 存放在 `.scratch/`(已忽略,不入库)。见 `docs/agents/issue-tracker.md`。
### Triage labels
使用默认 label 词汇(needs-triage / needs-info / ready-for-agent / ready-for-human / wontfix / resolved,bug / enhancement)。见 `docs/agents/triage-labels.md`。
### Domain docs
单一上下文:根目录 `CONTEXT.md` + `docs/adr/`。见 `docs/agents/domain.md`。
