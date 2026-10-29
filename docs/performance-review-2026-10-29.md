# 性能与内存审查（2026-10-29）

已修复本轮确认的 6 项问题，保留磁盘校验、取消归属、事务、逐片日志和唯一索引写入口。测试与测量只使用临时目录及中性合成数据。

## 固定范围与证据

- 目标：`2547e9a327d596255eb846ffdbcccc65e1f26a0a → 优化前 working tree`，包含 52 个已修改文件与 31 个新增普通文本文件，excluded 0。
- 快照 SHA-256：`d2ccce5eb4ec76f1a61f4661afa5628044d71904a57ecefd42acda7a09e32289`。两个审查轴使用同一份只读快照；汇总前复核 HEAD、聚合 diff、untracked manifest、文件哈希均未漂移，随后开始修复，结束后删除临时审查快照。
- Standards 来源：AGENTS、CLAUDE、CONTEXT、ADR-0001、ADR-0006，以及 code-review skill 的 REVIEW-BASELINE。资源问题均经触发路径或实测复核。
- Spec 来源：[媒体库浏览](library-browser.md)、[第二批工作流](workflow-round2.md)、CONTEXT 的分批检查与索引分页约束，以及本轮用户的性能和内存优化要求。

## Standards

| 级别 | 优化前位置 | 场景与影响 | 已完成的修复 |
|---|---|---|---|
| blocking | `apps/desktop/src/lib/overview-requests.ts:6` | 队首查询迟迟不返回时，Promise 链保留已取消查询和组件回调 | 可移除等待集合，abort 立即 reject 并释放引用；实际 transport 仍单并发 |
| blocking | `apps/desktop/src-tauri/src/commands/batch.rs:80` | 关闭批量编辑后，完整预览和目录句柄仍被原生槽持有 | token 定向 discard，覆盖返回、关闭、卸载、设置失效、保存失败及迟到取消；开始新预览前释放旧批次 |
| suggestion | `apps/desktop/src-tauri/src/commands/batch.rs:73` | 为仅展示差异的界面 clone 完整 before/after 与不参与序列化的写入字节，再传输全部正文 | 独立轻量 BatchEditPreview；完整预览只留原生，前端使用 raw state |
| suggestion | `apps/desktop/src/views/Library.svelte:94` | 展开实时 query 后覆盖 text，仍导致每按键产生新对象并刷新列表/概览 | 用完整防抖查询的序列化值稳定派生对象身份 |
| suggestion | `crates/kiroku-core/src/library/query.rs:218` | 反向年份/评分的空值 ASC、数值/编号 DESC 无匹配索引，首屏和深跳使用临时排序 | SQLite v5 增加两条混合方向索引；旧索引原地迁移，保持结果及空值语义 |
| suggestion | `crates/kiroku-core/src/library/prune.rs:22` | 带 nullable OR 的分页条件反复扫描此前编号，累计扫描近似 O(N²/256) | 首批无游标，后续直接 `code > ?`，保留每批 256 条、事务与所有校验 |

Standards：blocking 2、suggestion 4，均已解决。最严重的是取消队列的持续保留与放弃批量预览后的原生资源驻留。

## Spec

Spec：blocking 0、suggestion 5，均已解决，分别对应概览取消、防抖、检查分页、双向排序和批量预览资源。批量响应复制与关闭释放在该轴合并为一项；没有把未明确承诺的内存预算当作功能违约。两批功能契约继续成立。

## 测量

以下为本机开发构建、独立合成场景。各性能实验顺序运行；时间不是硬件无关门限。

| 场景 | 修复前 | 修复后 |
|---|---:|---:|
| 活跃概览挂起，1000 个 16 KiB 查询逐个取消，GC 后堆增量 | 17,052,432 B；0 个结束 | 2,572 B；1000 个立即结束 |
| 13 字符、40 ms 间隔输入，工具已展开 | 初始查询后额外 14 次概览 | 初始查询后 1 次最终概览；列表同样 1 次 |
| 200 部影片，每部 64 KiB 合成正文，批量响应 JSON | 26,509,696 B | 19,696 B（减少约 99.93%） |
| 同一原生响应实验，独立进程峰值 RSS | 229,457,920 B（218.8 MiB） | 128,614,400 B（122.7 MiB），降低约 44% |
| 原生预览持有/释放文件描述符 | 预览后驻留，界面没有释放入口 | 基线 4 → 持有 405 → discard 后 4 |
| 十万行，256 条一批，纯 SQL 编号遍历 | 1539 ms | 39 ms |
| 十万行，反向年份首屏 120 条，5 次中位数 | 19.013 ms | 0.125 ms |
| 同上，冷跳 offset 80000 | 196.559 ms | 2.923 ms |

查询对比使用项目 bundled SQLite 3.53.2、相同内存库和旧/新 SQL 或索引，包含重复值与空值，不含磁盘目录检查。prune 的实际总耗时还取决于文件系统；不能把纯 SQL 加速倍数当成完整刷新加速倍数。

原生 RSS 对比使用同一测试程序的 `legacy` 和 `compact` 两种响应构造方式，各自启动独立进程，以 macOS `/usr/bin/time -l` 读取峰值。包含 200 个实际磁盘修订预览，旧模式重现完整 clone 和 JSON 序列化；这是测试进程峰值，未包含 Tauri WebKit、图片解码或长期使用后的分配器状态。释放句柄以 `/dev/fd` 实测。

相同独立浏览器脚本的取消实验从 17,052,432 B 降到 2,572 B；自动回归中另一次测得 27,376 B，均远低于 2 MiB 回归门限。堆数字受浏览器暖机状态影响，不作为精确常量。

另通过真实 `reindex_dir` 写入十万条合成记录，再使用 readonly reader 测量：反向年份首屏中位数 0.772 ms、offset 99880 深页 5.691 ms、反向评分深页 5.965 ms（各 10 次）。该 fixture 复用一个临时目录来构造索引，不代表十万个磁盘目录的重建耗时。

海报墙原有长滚动基线仍通过：100000 个合成条目，缓存上限 6 页/720 卡片；预热 12 页后滚到第 48 页，GC 后堆增量 370,404 B。没有据此改动现有缓存预算。

## 验证与复现

- `cargo test`：576 passed，22 ignored；新增最大批量测量另行执行通过。
- `pnpm exec playwright test`：322 passed，包含搜索派发次数、队首挂起时取消/回收、预览关闭/返回/离页、迟到响应、旧 token 和保存前失败。
- `pnpm check`：0 errors / 0 warnings；`pnpm build`、`cargo check -p kiroku-desktop` 与 `git diff --check` 通过。
- 核心回归验证旧索引迁移、两种 nullable sort 的两个方向、冷页/深页不使用临时排序、有锚点范围查找、跨批编号不重不漏，以及既有取消/根掉线回滚。
- 原生回归验证轻量响应不含 before/after 正文、旧 token 无法消费新预览、匹配 token 可释放整个槽。

可重复执行：

```sh
# 独立的原生响应与句柄测量；峰值 RSS 应直接运行 cargo 输出的测试可执行文件，避免计入编译器。
KIROKU_RESPONSE_MODE=legacy cargo test -p kiroku-desktop --lib batch_response_size_measurement -- --ignored --nocapture --test-threads=1
KIROKU_RESPONSE_MODE=compact cargo test -p kiroku-desktop --lib batch_response_size_measurement -- --ignored --nocapture --test-threads=1
cargo test -p kiroku-core --lib large_library_query_measurement -- --ignored --nocapture --test-threads=1
cd apps/desktop
pnpm exec playwright test tests/performance-lifecycle.spec.ts tests/poster-cache.spec.ts --workers 1
```

## 未确认项与保留边界

- 概览 CTE 的 `SELECT *` 没有证据表明会物化全部正文。bundled SQLite 的执行计划内联 CTE，窄投影对比约 122/121 ms，未据此修改 SQL。
- 没有把 `map → skip` 推断成所有被跳过项都会执行映射；历史详情仍接受单个 Journal 模型 O(N) 的既定边界。
- 批量预览打开期间，原生仍保留应用所需的完整元数据/写入字节，内存随最多 200 部的正文大小增长；本轮减少副本并修复生命周期，没有引入虚假的绝对内存上限。
- 新增索引占用额外磁盘空间，并在旧库首次升级时构建；随机冷跳仍有 OFFSET 成本。
- 已有全局主题监听属于应用生命周期；未发现新增监听/ResizeObserver 缺少卸载清理。没有宣称整个原生 WebKit 进程绝无泄漏，也未测真实超大库、网络盘或长时间运行的总 RSS。
