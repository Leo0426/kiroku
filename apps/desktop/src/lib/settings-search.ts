import type { SettingsTab } from '../views/Settings.svelte'
export type SettingLink = { tab: SettingsTab; label: string; keywords: string; target?: string }
/** Search metadata never contains settings values (tokens, cookies or private paths). */
export const SETTING_LINKS: SettingLink[] = [
  { tab:'general', label:'媒体库位置与重建索引', keywords:'目录 路径 缓存' },
  { tab:'general', label:'主题', keywords:'外观 浅色 深色 跟随系统 theme', target:'appearance-theme' },
  { tab:'general', label:'文字大小', keywords:'外观 字体 缩放 font', target:'appearance-text' },
  { tab:'general', label:'动态效果', keywords:'外观 动画 减少运动 motion', target:'appearance-motion' },
  { tab:'organize', label:'整理方式', keywords:'移动 复制 硬链接 软链接', target:'organize-mode' },
  { tab:'organize', label:'清理原文件夹、下载剧照与完成通知', keywords:'废纸篓 系统提醒 图片' },
  { tab:'naming', label:'目录模板', keywords:'命名 文件夹 占位符', target:'tpl' },
  { tab:'naming', label:'文件名模板', keywords:'命名 视频 占位符', target:'file-tpl' },
  { tab:'naming', label:'NFO 语言', keywords:'中文 原文 翻译 导出', target:'nfo-language' },
  { tab:'scan', label:'样片与预告片', keywords:'扫描 正则 过滤 sample', target:'scan-extra' },
  { tab:'scan', label:'扫描黑名单', keywords:'广告 跳过 排除', target:'scan-black' },
  { tab:'scan', label:'最小体积', keywords:'扫描 大小 MB 视频', target:'scan-min' },
  { tab:'names', label:'名称规则', keywords:'别名 归一 标签 制作商 发行商 系列' },
  { tab:'badges', label:'封面角标', keywords:'水印 图片 字幕 位置 大小', target:'badge-scale' },
  { tab:'sources', label:'数据源与认证', keywords:'站点 域名 cookie 登录 启用 检查' },
  { tab:'sources', label:'请求间隔', keywords:'刮削 速率 节流', target:'interval' },
  { tab:'sources', label:'全局代理', keywords:'网络 proxy', target:'proxy' },
  { tab:'sources', label:'求解器', keywords:'网络 solver 反爬', target:'solver' },
  { tab:'ai', label:'AI 汉化', keywords:'翻译 模型 提示词 provider OpenAI Anthropic API key 缓存' },
  { tab:'plex', label:'Plex', keywords:'媒体服务器 命名 token 刷新', target:'plex-url' },
  { tab:'plex', label:'Jellyfin 与 Emby', keywords:'媒体服务器 API key 刷新', target:'server-kind' },
  { tab:'plex', label:'媒体服务器路径映射', keywords:'本机 远程 前缀 NAS', target:'server-local' },
  { tab:'data', label:'生成数据清理', keywords:'维护 备份 原图 空间 保留 日志' },
  { tab:'advanced', label:'路线与字段优先级', keywords:'刮削 路由 聚合 屏蔽 数据源', target:'settings-routes' },
  { tab:'advanced', label:'运行日志', keywords:'诊断 错误 级别 文件 控制台' },
]
export function findSettings(text: string) {
  const terms = text.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean)
  return terms.length ? SETTING_LINKS.filter(item => terms.every(term => `${item.label} ${item.keywords}`.toLocaleLowerCase().includes(term))) : []
}
