# ADR-0010：Jellyfin/Emby 局部刷新（修订 ADR-0001 范围）

- 状态：接受（2026-10-15 已实现）
- 日期：2026-10-15
- 关联：ADR-0001（把范围里的「Plex 联动」扩展为「媒体服务器联动」）

## 问题与约束

kiroku 写的 NFO、海报和背景图主要给 Jellyfin/Emby 用，却只能通知 Plex。整理完以后，用户要等 Jellyfin/Emby 的定时扫描，或者手动点刷新。amane #72 和 PR #374、MDC 8.1.4 都在做和 Jellyfin/Emby 的联动。

约束：单进程；不做服务化外壳；只发出站请求；凭据不写进日志和快照。

## 决策

- 新增 `media_server` 模块（Jellyfin/Emby 共用），设置 `media_server` 与 `plex` 并列，可同时启用；桌面端 `notify_servers` 在整理和撤销后依次通知。没有为三种服务器再抽 trait：Plex 按分区刷新、Jellyfin/Emby 按路径通知，形状不同，抽象收益小。
- Jellyfin/Emby 只发按路径通知：`POST /Library/Media/Updated`，body 为 `{"Updates":[{"Path":..., "UpdateType":"Created|Modified|Deleted"}]}`。整理传新目录，撤销传原目录。鉴权按服务器类型选 `Authorization: MediaBrowser Token="…"` 或 `X-Emby-Token`，不把 key 放进 URL。已对照 Jellyfin 官方 OpenAPI 12.2.0 核实（`MediaUpdateInfoDto.Updates[].Path/UpdateType`，成功 204）；Emby 使用同名接口，尚无一手文档核实。
- 设置页提供「测试连接」：读 `/System/Info` 和 `/Library/VirtualFolders`，标出包含媒体库目录的服务器媒体库。
- 可选路径前缀映射（本机前缀 → 服务器前缀，按路径组件匹配），用于服务器跑在 Docker/NAS 上、看到的路径不同的情况。
- 通知失败只提示，不影响整理结果。

## 不做

- 推送演员人物信息（amane #374 的范围）。
- 从服务器读回元数据。
- 写观看状态。
- 管理多个服务器分区。

## 验证

- 用 localhost 模拟服务器测请求格式、鉴权头和错误映射。
- 用真实 Jellyfin 和 Emby 各验证一次：整理后几秒内出现新条目，撤销后条目消失。

## 代价

- 要维护两种服务器的 API 差异。
- ADR-0001 的范围变宽，但仍然只是出站 HTTP。

## 实现记录（2026-10-15）

- core `media_server`：`MediaServerCfg`（kind/url/api_key/local_prefix/server_prefix）、`updates`（存在 → Created，已删 → Deleted）、`test`、`notify`；错误按 401/403/其他状态分类，不含 API Key，Key 只放在请求头。
- 桌面端 `media_server_test` 命令与 `server:refreshed` 事件（与 `plex:refreshed` 共用 toast）；设置「媒体服务器」页新增 Jellyfin/Emby 卡片。
- 测试：localhost 模拟服务器校验请求方法、路径、鉴权头、请求体和错误映射；浏览器 mock 覆盖设置页的测试连接与路径映射。真实 Jellyfin/Emby 尚未实测。
