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

export function applyNotice(d: ApplyDone, filesWithNotes: number): Notice {
  let title = '整理完成'
  if (d.cancelled) title = '整理已停止'
  else if (d.failed.length) title = d.applied ? '整理部分完成' : '整理未完成'
  else if (d.indexError) title = '媒体库索引待更新'
  else if (d.imageFailures) title = '整理完成，有图片待补'

  // 同一计划项可以先写入视频，再因保存资料失败而上报错误。
  const parts: string[] = []
  if (d.applied || d.cancelled || d.failed.length) parts.push(`${d.applied} 项已写入视频或字幕`)
  if (d.failed.length) parts.push(`${d.failed.length} 条执行错误`)
  if (d.skipped) parts.push(`${d.skipped} 项未选或有冲突`)
  if (d.imageFailures) parts.push(`${d.imageFailures} 张图片准备失败`)
  if (d.indexError) parts.push('索引更新失败')
  if (filesWithNotes && !d.imageFailures) parts.push(`${filesWithNotes} 项有文件提示`)
  return { title, body: parts.join('，') || '文件操作已完成' }
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
