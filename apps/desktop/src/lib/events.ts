import type { EventMap } from './types'

export const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

type Handler<K extends keyof EventMap> = (payload: EventMap[K]) => void

// 浏览器预览时,mock 引擎通过这条总线发事件
const bus = new EventTarget()

export function emitMock<K extends keyof EventMap>(event: K, payload: EventMap[K]) {
  bus.dispatchEvent(new CustomEvent(event, { detail: payload }))
}

/** 订阅后端事件;返回取消订阅函数。Tauri 与浏览器 mock 同一接口。 */
export async function listen<K extends keyof EventMap>(event: K, handler: Handler<K>): Promise<() => void> {
  if (inTauri) {
    const { listen: tlisten } = await import('@tauri-apps/api/event')
    return tlisten<EventMap[K]>(event, (e) => handler(e.payload))
  }
  const fn = (e: Event) => handler((e as CustomEvent).detail)
  bus.addEventListener(event, fn)
  return () => bus.removeEventListener(event, fn)
}
