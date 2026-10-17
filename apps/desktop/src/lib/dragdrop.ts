import { inTauri } from './events'

/** 与平台无关的窗口拖放:进入/离开控制提示层,放下给出路径。 */
export type PathDrag = { type: 'enter' } | { type: 'leave' } | { type: 'drop'; paths: string[] }

/**
 * 订阅拖入窗口的文件路径;返回取消订阅函数。
 * Tauri 里用原生窗口拖放;浏览器预览没有真实路径,用 `kiroku-mock-drag` 窗口事件模拟。
 */
export async function onPathDrag(handler: (e: PathDrag) => void): Promise<() => void> {
  if (inTauri) {
    const { getCurrentWebview } = await import('@tauri-apps/api/webview')
    return getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (payload.type === 'enter') handler({ type: 'enter' })
      else if (payload.type === 'leave') handler({ type: 'leave' })
      else if (payload.type === 'drop') handler({ type: 'drop', paths: payload.paths })
    })
  }
  const fn = (e: Event) => {
    const { type, paths } = (e as CustomEvent<{ type: string; paths?: string[] }>).detail
    if (type === 'enter' || type === 'leave') handler({ type })
    else if (type === 'drop') handler({ type: 'drop', paths: paths ?? [] })
  }
  window.addEventListener('kiroku-mock-drag', fn)
  return () => window.removeEventListener('kiroku-mock-drag', fn)
}
