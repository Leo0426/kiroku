# 命名模板与 NFO 文本语言

目录与文件模板保留原默认输出、值映射和可选组。设置页实时样例使用示例文件上下文；整理预览使用真实来源文件，显示截短及语言缺失提示。

新增变量：

| 变量 | 值 |
| --- | --- |
| prefix / suffix | 规范编号的前后部分，保留前导零 |
| actors / actress / actresses | 全部演员 / 首个女演员 / 全部女演员；列表稳定去重并用「、」连接 |
| release / runtime / score | 来源日期 / 分钟 / 各站平均评分 0–100 |
| original_title / directors | 已知原题 / 导演列表 |
| raw_name / raw_dir / ext | 来源文件无扩展名名称 / 父目录名称 / 不带点的真实扩展名 |

actor 仍为首个女演员，否则首个演员。没有文件上下文的目录渲染不会猜测来源文件。ext 可用于目录模板，文件名模板必须省略，整理时唯一附加。新保存的模板检查未知变量、括号与映射；旧未知变量保持渲染兼容。文件名必须有真正的 code/number 节点；不能把 codefake 或映射替换成其他值当作编号。

每个目录分量和完整文件名最多 240 UTF-8 字节，包含扩展名。截短保留编号和分片标记，无法保留时预览冲突；截短后重名仍阻止执行。不能使用绝对路径或 ..。追加到已有影片若目标名超过限制，提示冲突而不改动已有视频。

## 文本版本

kiroku.json 保存 text_versions.original 与 localized（BCP 47 标签）的 title/plot，以及各字段来源。当前扁平 title/plot 继续用于界面与索引。AI 仍仅生成 zh-Hans，翻译前保存实际采用的来源原文；没有增加翻译结果缓存。详情可保存用户提供的其他语言版本，或选择已有版本为当前文本。人工锁定字段，包括锁定空值，始终优先。

旧 sidecar 不根据汉字猜测语言。只有明确 original_title 或采用来源的 raw 快照才可恢复原文；不能可靠恢复时显示「原文未保存」。所选字段重新抓取或来源重选只失效对应字段的旧版本；未更新字段保持已有原文与译文。

## NFO 导出

设置提供 current（默认）、zh-Hans、original，用于新整理计划；保存设置不会改写已有影片。详情页可逐片预览/确认导出，媒体库可选择影片批量预览/应用；所有操作都有历史日志可撤销，保持影片目录和视频原位。缺失对应字段版本时回退当前值并提示。每个 NFO 使用一个 title 与一个 plot；已知 originaltitle 保存真实原题，未知时省略。标题空缺以编号满足 NFO 可用性，sidecar 的人工空值不变。

语言导出只更新语言节点与 sidecar，保留其他 XML、属性和播放记录。只生成 movie.nfo，不写竞争的 movie.zh.nfo 或重复带 lang 属性的 title。Plex 仍靠文件名与 poster/fanart，不读取 NFO。

Kodi 使用 movie.nfo 需要启用「影片位于匹配标题的独立目录」，见 [Kodi 官方说明](https://kodi.wiki/view/NFO_files/Movies)。本次未进行真实 Kodi/Emby/Jellyfin 播放服务器兼容验收，XML 回归测试不能替代服务器验收。

CLI 默认预览：

```sh
kiroku nfo-export /library/Actor/ABC-001 --root /library --language original
kiroku nfo-export /library/Actor/ABC-001 /library/Actor/ABC-002 --root /library --language zh-Hans --apply
kiroku rollback /library/.kiroku/journal-<id>.json
```
