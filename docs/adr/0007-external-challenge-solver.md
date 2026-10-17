# ADR-0007：经用户自行运行的求解器访问 Cloudflare 挑战站点

- 状态：接受
- 日期：2026-10-14
- 关联：ADR-0001

## 问题与约束

部分数据源对普通请求返回 Cloudflare「Just a moment…」挑战页（403）。ADR-0001 定下单进程桌面应用、不做服务化外壳；PoC 也没有引入浏览器内核或 TLS 指纹伪装。amane 等同类工具的做法是调用用户运行的 FlareSolverr。

## 决策

- kiroku 不打包、不下载、不启动求解器，只调用用户在设置里填写的 FlareSolverr v1 地址（`POST <地址>`，`cmd: request.get / request.post`）。没有填地址时这项能力不存在。
- 按来源开启，默认关闭。开启后该来源的文本请求（详情、搜索）改走求解器；图片仍由 kiroku 直接下载，不经过求解器。
- 同站请求间隔、重试策略和来源熔断不变，按目标站点 host 计算；求解器自己的耗时不计入间隔。来源的代理设置作为求解器请求的 `proxy` 传过去，由求解器出站。
- 求解器返回的 Cookie 并入该来源的 Cookie 会话；会话里已有的 Cookie（包括用户填写的）和来源请求里的 `Cookie` 头（例如站点的访问确认 Cookie）随请求交给求解器。FlareSolverr 不支持自定义请求头，其他请求头在走求解器时不生效。
- 求解器报错（`status` 不是 `ok`）映射为 502 页面交给来源，按网络失败处理并进入熔断计数；不会退回直连重试，避免在挑战页上反复请求。

## 验证

2026-10-15 用 FlareSolverr 3.5.2（本机 Docker）实测：两个受挑战保护的数据源都能经求解器返回 200，kiroku 的 `Http` + `Solver` 端到端取回完整字段（`tests/live_sources.rs`，需设 `KIROKU_SOLVER`）。回复格式的合成样例在 `tests/fixtures/flaresolverr/`。

## 代价

- 依赖用户自己维护一个外部进程，kiroku 只负责报清楚错误。
- 求解器每次请求通常要数秒到数十秒，开启的来源会明显变慢。
- 除 Cookie 外的请求头不能透传：依赖其他请求头的来源（例如需要特定 Referer 或 JSON 请求体的 GraphQL 接口）不应开启。
- 求解器拿到的 `cf_clearance` 绑定它自己的浏览器 UA，kiroku 直连的图片请求不能复用它；图片站本身被挑战时仍会失败。
