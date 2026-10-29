# 媒体库浏览改进

2026-10-29 完成。本轮围绕查找、浏览、核对和导出新增 10 项功能；保留单进程桌面架构，媒体磁盘文件仍是唯一真相。

## 调研与取舍

[Plex Library View](https://support.plex.tv/articles/200392126-using-the-library-view/) 描述网格/列表、海报尺寸、双向排序和当前视图随机播放；[Plex Collections](https://support.plex.tv/articles/201273953-collections/) 描述由筛选与排序构成的智能集合。本轮据此选择保存查询、可切换的浏览方式和筛选内随机挑选；随机只打开详情。Plex 页面直接访问返回 403，功能信息来自同一官方 URL 的搜索索引正文，未声称实机验证其现版界面。

[tinyMediaManager Filters](https://www.tinymediamanager.org/docs/movies/filters) 提供缺资料/图片筛选，[Export Templates](https://www.tinymediamanager.org/docs/exporttemplates) 支持 CSV 等清单导出。本轮复用已有编辑、补图和健康工作流，增加对应的检索入口与只读导出。最近搜索、位置恢复和命令面板是减少重复操作的设计选择，没有把已有的收藏、健康检查、撤销或编辑能力重复计数。

## 10 项功能及验收对应

| # | 功能 | 完成行为 | 验证证据 |
|---|---|---|---|
| 1 | 命名筛选视图 | 保存、恢复、更新、删除完整查询；按库隔离，重复名称提示 | `library-browser.spec.ts` 的 named views、preferences isolate |
| 2 | 最近搜索 | 提交/离开搜索框时记录，去重，上限 8 条；复用与清除 | recent searches 回归 |
| 3 | 列表与海报密度 | 虚拟列表、三档海报；模式与大小记忆，共用分页/缓存预算 | list mode 回归；原有十万条海报缓存回归 |
| 4 | 双向排序 | 四种排序键均支持反向，服务端分页前排序，同值编号稳定排序，空值始终最后 | core `cached_counts_and_seek_pages_match_all_sorts_filters_and_nulls` 覆盖两方向与全部排序键 |
| 5 | 筛选内随机挑选 | 从全部匹配项选取，不限已加载页；等待查询或空结果时禁用；旧响应无效 | random unloaded result、late random selection 回归 |
| 6 | 详情连续浏览 | 前后导航跨页，首末边界禁用；编辑/校验时禁用；库变化后旧位置失效，关闭归还焦点 | continuous detail navigation、old position 回归；原有媒体取消测试 |
| 7 | 完整性和评分筛选 | 缺海报/评分/年份、最低评分，与既有条件 AND 组合；同时作用于总数和导出 | core completeness filters 与浏览器 filters 回归 |
| 8 | 浏览状态恢复 | 保存查询、布局和滚动位置，跨页面和重载恢复，按库隔离，损坏数据降级 | list mode、preferences isolate 回归 |
| 9 | 全局命令面板 | Ctrl/⌘ K、搜索操作、↑↓/Enter/Esc、页面快捷键与帮助；沿用导航保护，原生 dialog 管理模态焦点 | command palette 回归与既有导航测试 |
| 10 | CSV 清单导出 | 完整快照、进度、按任务停止；新文件独占发布，错误/取消不留下半份文件，不改媒体 | core 505 行跨页/并发更新快照测试；desktop publication 测试；浏览器完整下载及实际 mock 取消测试 |

## 布局与交互

工具栏将次要筛选和视图管理折叠，标签与显示控制放在同一行。窄窗口按可读的最小控件宽度折行，展开较多选项时头部独立滚动并保留结果区。列表与海报共用容器尺寸观察、虚拟化和请求预算；加载反馈、失败重试及结果数状态保留。详情防误触与系统减少动态效果偏好继续生效。

实查深/浅色桌面、列表、极窄筛选和命令面板截图；自动布局检查覆盖 320、480、720、960、1280、1920 像素，另保留原有各侧栏断点回归。极窄下拉框增加最小宽度，避免只剩箭头。该检查不等于对整个应用作 WCAG 合规声明。设计参照 [W3C Reflow](https://www.w3.org/WAI/WCAG22/Understanding/reflow.html)、[WAI Modal Dialog](https://www.w3.org/WAI/ARIA/apg/patterns/dialog-modal/) 和 [MDN reduced motion](https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/At-rules/@media/prefers-reduced-motion)。

## 数据与边界

- 本地偏好保留最近 10 个库，每库最多 20 个视图和 8 条搜索；不影响媒体文件与共享设置。库根改名视为另一库，WebView 存储清除后偏好也会清除。
- 完整性依据索引投影，不实时逐个检查图片；磁盘异常仍使用文件健康检查。评分界面采用 0–10，内部采用 0–100。
- CSV 字段为编号、标题、年份、演员、评分、文件数、收藏。输出带 UTF-8 BOM 和 CRLF，逐格引号转义、中和公式前缀。遵循 [OWASP CSV Injection](https://community.owasp.org/attacks/CSV_Injection) 的边界：不保证任意表格软件后续反复保存或重新解释仍保留同一保护。
- 原生先选新文件名，再占任务槽做只读快照导出；拒绝覆盖已有文件。未完成数据写私有临时文件，发布前同步并复验。选择器本身不持有退出 guard，避免等待用户输入阻止应用退出。
- 未新增网络服务、文件监控或播放状态数据库；未访问真实媒体或系统废纸篓。

## 检查记录

- `cargo test`：566 passed、21 ignored。其后新增原生 CSV 发布测试，再运行 `cargo test -p kiroku-desktop --lib`：40 passed；其中覆盖成功写盘、拒绝覆盖、取消清理及旧库快照失效。
- `cargo test -p kiroku-core library:: --lib`：33 passed、4 ignored；含双向分页、完整性、505 行 CSV 快照与取消。
- `pnpm exec playwright test`：301 passed。之后窄窗口与结果反馈调整，23 条布局/浏览回归通过；增加旧位置保护后，16 条浏览/详情/收藏回归通过；最后补齐命令面板 Tab 循环、关闭按钮 Enter、中文组合输入和未保存设置保护，21 条浏览/导航/设置回归通过。
- 最终 `pnpm check`：0 errors、0 warnings；`pnpm build`、`cargo check -p kiroku-desktop`、`git diff --check` 通过。

上述写盘验证使用临时目录中的真实 Rust 实现，界面使用浏览器 mock。原生系统保存对话框的人工操作、完整 WebKit 导出桥接和 Windows 实机本轮未验收。
