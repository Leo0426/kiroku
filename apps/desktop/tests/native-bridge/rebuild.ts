import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { mount, tick, unmount } from 'svelte'
import Toast from '../../src/components/Toast.svelte'
import { api } from '../../src/lib/api'
import { app } from '../../src/lib/state.svelte'
import type { ProgressEvent, RebuiltEvent } from '../../src/lib/types'

type Snapshot = { jobId: number | null, cancelled: boolean, cancels: number[], notifications: { title: string, body: string }[] }
const snapshot = () => invoke<Snapshot>('bridge_snapshot')
function assert(condition: unknown, message: string): asserts condition { if (!condition) throw new Error(message) }
async function until(predicate: () => boolean | Promise<boolean>, message: string) {
  const deadline = Date.now() + 5000
  while (!await predicate()) {
    if (Date.now() >= deadline) throw new Error(`Timed out: ${message}`)
    await new Promise(resolve => setTimeout(resolve, 10))
  }
}

async function index(expected: 'old' | 'rebuilt') {
  const page = await api.libraryList({ limit: 10 })
  assert(page.total === (expected === 'old' ? 2 : 3), `Wrong ${expected} index size: ${page.total}`)
  assert((await api.libraryGet('SMPL-001'))?.meta.title === (expected === 'old' ? 'Localized sample' : 'Rebuilt sample'), `Wrong ${expected} original title`)
  assert((await api.libraryGet('SMPL-002'))?.meta.title === 'Localized sample', 'Unchanged film lost from index')
  const added = await api.libraryGet('IDX-003')
  assert(expected === 'old' ? added === null : added?.meta.title === 'New index sample', `Wrong ${expected} added-film projection`)
}

export async function runRebuildChecks(passed: string[]) {
  await app.init()
  await until(() => !app.recovery.loading, 'fixture recovery read')
  assert(!app.recovery.error && app.settings?.notify_on_finish === false, 'Fixture initialization or notification isolation failed')
  const root = app.settings.library_root
  await index('old')
  await invoke('bridge_rebuild_prepare')
  await index('old') // The fixture edits disk only; the cache must still be stale.
  const before = await snapshot()
  const completed: RebuiltEvent[] = []
  const progress: ProgressEvent[] = []
  let waiting: ProgressEvent | null = null
  const stopDone = await listen<RebuiltEvent>('library:rebuilt', event => completed.push(event.payload))
  const stopProgress = await listen<ProgressEvent>('library:progress', event => progress.push(event.payload))
  const stopWaiting = await listen<ProgressEvent>('bridge:rebuild_waiting', event => { waiting = event.payload })
  const host = document.createElement('div')
  document.body.append(host)
  const component = mount(Toast, { target: host })
  for (const toast of app.toasts) app.dismiss(toast.id)
  const start = api.libraryRebuild
  let releaseResponse: () => void = () => {}
  const responseGate = new Promise<void>(resolve => { releaseResponse = resolve })
  let id: number | null = null
  // Keep the actual IPC/result, but let its native completion reach AppStore first.
  api.libraryRebuild = async () => {
    const result = await start()
    id = result
    await responseGate
    return result
  }
  try {
    const started = app.rebuildIndex()
    await until(() => id !== null && waiting !== null, 'first native rebuild directory')
    const held = waiting as ProgressEvent | null
    assert(held?.jobId === id && held.root === root && held.done === 1 && held.total === 3, 'Wrong transaction hold point')
    assert(await api.jobCancel(id!), 'Native rebuild rejected scoped cancellation')
    const cancelled = await snapshot()
    assert(cancelled.jobId === id && cancelled.cancelled && cancelled.cancels.at(-1) === id, 'Cancellation did not reach the held transaction')
    await invoke('bridge_rebuild_release')
    await until(() => completed.some(event => event.jobId === id), 'native cancelled rebuild completion')
    const done = completed.find(event => event.jobId === id)!
    assert(done.root === root && done.cancelled && !done.error && done.count === 1, 'Cancelled rebuild did not stop after its first indexed directory')
    assert(progress.some(event => event.jobId === id && event.done === 1 && event.total === 3), 'Real rebuild progress did not cross WebKit')
    assert(app.rebuild.running && app.toasts.length === 0, 'Early completion claimed ownership before the start response')
    assert((await snapshot()).notifications.length === before.notifications.length, 'Early completion notified before ownership')
    releaseResponse()
    await started
    await tick()
    assert(!app.rebuild.running && host.textContent?.includes('已取消重建，保留原索引'), 'Cancelled rebuild did not render through real Toast')
    await until(async () => (await snapshot()).notifications.length === before.notifications.length + 1, 'cancel notification request')
    await index('old')
    await invoke('bridge_rebuild_verify')
    passed.push('rebuild_cancel_transaction')

    api.libraryRebuild = start
    await app.rebuildIndex()
    await until(() => !app.rebuild.running, 'rebuild retry completion')
    await tick()
    assert(host.textContent?.includes('索引已重建:3 部影片'), 'Successful rebuild result did not render')
    await index('rebuilt')
    await invoke('bridge_rebuild_verify')
    await invoke('bridge_rebuild_restore')
    // Restore via the same real rebuild path, with no direct SQL/cache writes.
    await app.rebuildIndex()
    await until(() => !app.rebuild.running, 'restored fixture rebuild')
    await index('old')
    await invoke('bridge_verify', { journals: 3 })
    await invoke('bridge_rebuild_verify')
    await until(async () => (await snapshot()).notifications.length === before.notifications.length + 3, 'one notification request per completion')
    const after = await snapshot()
    assert(after.jobId === null && after.cancels.length === before.cancels.length + 1, 'Rebuild task leaked or cancelled an unrelated task')
    const expected = [
      ['索引重建已取消', '保留原索引'],
      ['索引重建完成', '共 3 部影片'],
      ['索引重建完成', '共 2 部影片'],
    ]
    assert(after.notifications.slice(before.notifications.length).every((notice, i) =>
      notice.title === expected[i][0] && notice.body === expected[i][1]), 'Wrong or duplicate native notification requests')
    assert(completed.length === 3 && completed.every(event => event.root === root), 'Wrong native completion count/root')
    passed.push('rebuild_retry_restore')
  } finally {
    releaseResponse()
    api.libraryRebuild = start
    await invoke('bridge_rebuild_release')
    stopDone(); stopProgress(); stopWaiting()
    await unmount(component)
    host.remove()
  }
}
