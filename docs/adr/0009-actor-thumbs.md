# ADR-0009：演员头像写进 NFO

- 状态：接受（2026-10-15 已实现第 1–3、5 条；第 4 条暂缓）
- 日期：2026-10-15
- 关联：ADR-0001

## 问题与约束

`nfo.rs` 本来就是给 Emby/Jellyfin/Kodi 读的，但 `<actor>` 只写了 `name` 和 `type`，导致媒体服务器的人物页没有头像。部分数据源的详情页已经给出演员头像，只是解析时丢掉了。amane 等同类工具都有这项能力。

约束：站点解析保持纯函数，用 fixture 做回归；Plex 不读 NFO。

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
