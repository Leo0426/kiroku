// 长任务结束时的系统通知文案;是否真的发出(设置、窗口是否在前台)由后端决定
import { api } from './api'
import type { ApplyDone, RebuiltEvent, ScrapeDone } from './types'

export interface Notice { title: string; body: string }

export function scrapeNotice(d: ScrapeDone): Notice {
  const parts = [`找到 ${d.found} 部`]
  if (d.notFound) parts.push(`未找到 ${d.notFound} 部`)
  if (d.failed) parts.push(`失败 ${d.failed} 部`)
  return { title: d.cancelled ? '刮削已取消' : '刮削完成', body: parts.join('，') }
}

export function applyNotice(d: ApplyDone): Notice {
  const parts = [`已整理 ${d.applied} 部`]
  if (d.failed.length) parts.push(`失败 ${d.failed.length} 部`)
  if (d.indexError) parts.push('索引更新失败')
  return { title: d.cancelled ? '整理已取消' : d.failed.length || d.indexError ? '整理完成，有失败项' : '整理完成', body: parts.join('，') }
}

export function rebuildNotice(e: RebuiltEvent): Notice {
  if (e.error) return { title: '索引重建失败', body: e.error }
  if (e.cancelled) return { title: '索引重建已取消', body: '保留原索引' }
  return { title: '索引重建完成', body: `共 ${e.count} 部影片` }
}

/** 通知失败不影响任务结果,只记到控制台 */
export function notifyFinish(n: Notice) {
  void api.notifyFinish(n.title, n.body).catch((e) => console.warn('系统通知失败', e))
}
