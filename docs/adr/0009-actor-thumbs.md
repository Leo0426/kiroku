# ADR-0009：演员头像写进 NFO

- 状态：接受（第 1–3、5 条已实现；第 4 条在 2026-11-15 按独立静态托管方案补充）
- 日期：2026-10-15
- 关联：ADR-0001

## 问题与约束

`nfo.rs` 本来就是给 Emby/Jellyfin/Kodi 读的，但 `<actor>` 只写了 `name` 和 `type`，导致媒体服务器的人物页没有头像。部分数据源的详情页已经给出演员头像，只是解析时丢掉了。amane 等同类工具都有这项能力。

约束：站点解析保持纯函数，用 fixture 做回归。Plex 的支持取决于版本与元数据代理，见下方修订。

## 决策

1. `metadata::Actor` 增加 `thumb: Option<String>`，用 `serde(default)` 保证旧 sidecar 能读。先在已给出头像的两个来源解析，其余站点以后按 fixture 逐个补。站点的 NoImage 占位图视为没有头像。
2. 聚合：按演员名在路由链上取第一个非空头像。名称规则改名后，头像跟随规范名。
3. NFO 按 Kodi 格式写 `<actor><name/><role/><order/><thumb/></actor>`。`order` 用演员在来源中的顺序。
4. 可选「下载演员头像」：**暂缓**。原稿写的「库根 `.actors/`」不准确：Kodi 约定的是每个影片目录下的 `.actors/`（空格换成下划线），Jellyfin 是否读取未核实。先用真实 Kodi/Jellyfin 验证 NFO `<thumb>` 远程地址的效果，再决定是否落地本地文件；落地时复用图片 Http 预算，已存在不覆盖，写入记进整理日志，并接入 health/maintenance 的引用检查。
5. 不内置第三方头像库：现有的头像库没有许可证，免责声明禁止商用。以后如果要做，只能是用户自填的头像仓库地址。

## 不做

- 演员档案页、简介、三围。
- 头像裁切。
- 向媒体服务器推送人物信息（另见 ADR-0010）。

## 验证

- 头像解析的 fixture 测试；旧 sidecar 兼容测试；NFO 快照测试。
- 用真实 Jellyfin 和 Kodi 验证人物图是否显示；验证结论回写本 ADR。

## 代价

- NFO 里是远程 URL：第 4 条落地前，离线或图床失效时媒体服务器没有头像。

## 实现记录（2026-10-15）

- `metadata::Actor.thumb`（`serde(default, skip_serializing_if = none)`，旧 sidecar 及名称投影记录字节不变）；`Actor::real_thumb` 过滤 nowprinting/noimage 占位图。
- HTML 来源从演员列表项同时取名字和头像（相对地址补成绝对地址）；JSON 来源取演员对象的 `image_url`。
- 聚合在演员名单定下之后，沿 actors 字段链按姓名回填头像；名称规则合并别名时保留先出现的头像。
- NFO 写 `<order>` 和 `<thumb>`；`nfo_read` 读回 thumb；修订合并沿用 actor 节点整体替换。
- 详情页演员标签显示头像（`referrerpolicy=no-referrer`，加载失败即移除）。

## 防盗链修复与 Plex NFO 支持（2026-11-15）

- 已复现：同一头像 URL 不带 Referer 返回 403，带同源 Referer 返回 200。旧详情页 `no-referrer` 直接加载因此失败。
- 详情页改为 `actor_image` 只读命令：只接受当前库影片已有的头像 URL，走 core `actor_image::fetch` 与共享 Http 图片通道，匹配来源时沿用其代理/Cookie。最多四个并发请求，每次网络加载限时 20 秒；响应限制 8 MiB，解码限制单边 4096 和 64 MiB，后台生成最大 256×256 PNG。界面使用 Blob URL，切换影片/库或销毁时释放，丢弃晚到结果，加载失败保留姓名。
- 不落盘、不改 NFO/sidecar、不引入 HTTP 服务。此修复仅解决 kiroku 显示，不能把进程内 Blob URL交给 Plex。
- 纠正旧兼容性判断：[Plex 官方 NFO 文档](https://support.plex.tv/articles/using-nfo-metadata-files-with-plex/)说明 PMS 1.43.1 起支持 NFO 代理及演员 `<thumb>` URL；[官方论坛说明](https://forums.plex.tv/t/plex-nfo-agent-forum-preview/936104?page=6)指出 NFO 图片只读取 URL，不读取本地图片路径。因此 `.actors/` 文件并不能直接解决 Plex 的防盗链问题。
- Plex 仍需可直接访问的图片 URL（例如用户已有的静态图片服务或不要求 Referer 的来源）。本轮未引入图片托管、未修改真实 Plex 配置，也未声称 Plex 头像已修复。
- 验证：core 模拟拒绝无 Referer 请求的图床，检查解码及缩放；前端回归拦截远程热链并返回 403，确认只显示后端返回的 Blob 图片，错误头像和缺图保留姓名。
- 本机真实库头像经新 core 加载路径成功返回可解码的 256×256 PNG；只读验证，无媒体文件改动。`pnpm check`、`pnpm build`、头像 Playwright 回归与 `cargo check -p kiroku-desktop` 通过。

## 可选头像托管（2026-11-15）

用户确认采用 NAS 稳定主机名 + 头像下载目录方案。本节修订上文「下载暂缓」：支持显式的单片头像发布，不内置静态服务，不自动上传第三方图床，也不写 Plex 数据库。

- `Settings.actor_hosting` 默认关闭，保存本机可访问的专用绝对目录和 HTTP(S) URL 前缀。URL 不含凭据、查询参数或片段。目录由用户先创建/挂载；不可用时拒绝，不能创建失联 NAS 的影子目录。主机名、端口和路由器 DNS 由用户配置。
- 设置 → 媒体服务器提供目录选择与 URL 前缀输入，包含静态服务映射和 Plex NFO 操作步骤。功能独立于 Plex 自动刷新开关，不要求 Plex Token。
- 详情 → 影片工具 → 演员头像：下载到内存并预览 → 明确确认发布 → 打开头像链接检查 → 在 Plex 刷新本片元数据。下载可取消，未配置/无来源头像/下载失败有明确提示。保存仍绑定设置、库、编号与磁盘快照，使用单任务槽。
- core `actor_hosting` 下载沿用图片 Http 的 Referer、代理、Cookie。输出规范 PNG，按内容 SHA-256 命名，重复文件校验后复用；同名不同内容和软链接拒绝覆盖。保存先独占发布缓存，再走既有 v3 修订更新 NFO 和 sidecar，索引仍只由 `Library::reindex_dir` 写入。
- `Metadata.hosted_actor_images` 按原始头像 URL 保存 `{file,url}`。`Actor.thumb` 和来源快照保留原 URL；NFO 导出投影到托管 URL。更改 URL 前缀后再次发布可从已验证缓存重新生成 URL，无需原站在线。显式发布会重写 actor 节点，其余 NFO 节点保留。
- 缓存是跨影片共享的不可变下载产物，不计入影片撤销删除。发布中断或元数据保存失败可能留下未引用缓存；不自动清理。撤销恢复原 NFO/sidecar，缓存保留，避免影响其他影片。保存设置本身不改现有影片，新导入也不自动发布头像。
- 覆盖验证：预览零磁盘写入、同源 Referer、缓存复用与换域名、原 URL 保留、NFO 未选节点保留、完整撤销、外部修改和失联目录拒绝；前端覆盖配置校验→预览→保存→撤销与未配置提示。真实 NAS 静态服务及 Plex 刷新结果需要用户配置后验证。
