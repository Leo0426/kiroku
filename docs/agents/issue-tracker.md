# Issue Tracker

此仓库的 issues 和 PRD 以 markdown 文件形式存放在 `.scratch/` 目录下。

## 约定

- 每个功能一个目录：`.scratch/<feature-slug>/`
- PRD 文件：`.scratch/<feature-slug>/PRD.md`
- 实现 issues：`.scratch/<feature-slug>/issues/<NN>-<slug>.md`，从 `01` 开始编号
- 类别记录在每个 issue 文件顶部附近的 `Category:` 行，值使用 `triage-labels.md` 中 `bug` 或 `enhancement` 对应的实际字符串
- Triage 状态记录在每个 issue 文件顶部附近的 `Status:` 行（role 字符串见 `triage-labels.md`）
- 评论和对话历史追加到文件底部的 `## 评论` 标题下

## 当 skill 说"发布到 issue tracker"时

在 `.scratch/<feature-slug>/` 下创建新文件（如需要则创建目录）。

## 当 skill 说"获取相关工单"时

读取引用路径处的文件。用户通常会直接传入路径或 issue 编号；按类别或状态筛选时分别查询 `Category:` 与 `Status:` 行，并用 `triage-labels.md` 反向解析规范 role。

## Wayfinding operations

`forgeflow:wayfinder` 的地图和 tickets 在本地 tracker 下的物理形态：

- **地图**：`.scratch/<effort-slug>/MAP.md`，顶部标注 `Labels: wayfinder:map`。正文即地图正文（Destination / Notes / Decisions so far / Not yet specified / Out of scope）。
- **子 ticket**：`.scratch/<effort-slug>/issues/<NN>-<slug>.md`，从 `01` 开始编号。顶部标注 `Labels: wayfinder:<type>`（`research` / `prototype` / `grilling` / `task` 之一）和 `Status:` 行。
- **认领**：issue 文件顶部的 `Assignee:` 行。非空即已认领；session 开始工作前先写入自己的标识。
- **Blocking**：本地 tracker 无原生依赖关系，使用正文约定——issue 顶部的 `Blocked-by: <NN>, <NN>` 行。所列编号全部关闭后该 ticket 即无阻塞。
- **Frontier 查询**：遍历 `issues/` 下 `Status:` 为开放、`Blocked-by` 全部已关闭、`Assignee:` 为空的文件，按编号排序。
- **解决**：答案写入 issue 底部的 `## Resolution` 章节，`Status:` 置为 `resolved`，并在 `MAP.md` 的 Decisions so far 追加一行指针（`[<标题>](issues/<NN>-<slug>.md) — <一行要点>`）。
