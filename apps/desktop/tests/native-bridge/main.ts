import { invoke } from '@tauri-apps/api/core'
import { listen as nativeListen } from '@tauri-apps/api/event'
import { api } from '../../src/lib/api'
import { cancellableJob } from '../../src/lib/cancellable-job'
import { inTauri } from '../../src/lib/events'
import { runNetworkChecks } from './network'
import { runImportUi } from './import-ui'
import { runRebuildChecks } from './rebuild'

type Result = { jobId: number, cancelled: boolean }
type Snapshot = { jobId: number | null, cancelled: boolean, starts: number, cancels: number[] }
type Started = { jobId: number, requestId: string }
const passed: string[] = []
const acquired: Started[] = []
const announced: Started[] = []
const cancelErrors: unknown[] = []
function assert(condition: unknown, message: string): asserts condition { if (!condition) throw new Error(message) }
const snapshot = () => invoke<Snapshot>('bridge_snapshot')
async function until(predicate: () => boolean, message: string) {
  const deadline = Date.now() + 5000
  while (!predicate()) {
    if (Date.now() >= deadline) throw new Error(`Timed out: ${message}`)
    await new Promise(resolve => setTimeout(resolve, 10))
  }
}
async function rejected(promise: Promise<unknown>, message: string) {
  try { await promise } catch (error) {
    assert(String(error).includes(message), `Expected ${message}, got ${String(error)}`)
    return
  }
  throw new Error(`Expected rejection: ${message}`)
}
const controlled = (requestId: string, delayed = false) => invoke<Result>('bridge_task', { requestId, delayed })

async function run() {
  assert(inTauri, 'Must run actual Tauri implementation')
  const stopAcquired = await nativeListen<Started>('bridge:acquired', event => acquired.push(event.payload))
  const stopAnnounced = await nativeListen<Started>('job:started', event => announced.push(event.payload))

  const before = await snapshot()
  let dispatched = false
  const immediate = cancellableJob(requestId => { dispatched = true; return controlled(requestId) }, error => cancelErrors.push(error))
  const immediateResult = rejected(immediate.promise, '取消')
  await immediate.cancel()
  await immediateResult
  assert(!dispatched && (await snapshot()).starts === before.starts, 'Early stop must not dispatch a command')
  passed.push('before_listener')

  let delayedRequest = ''
  const delayed = cancellableJob(requestId => { delayedRequest = requestId; return controlled(requestId, true) }, error => cancelErrors.push(error))
  await until(() => acquired.some(event => event.requestId === delayedRequest), 'native acquired before announcement')
  await invoke('bridge_foreign')
  await until(() => announced.some(event => event.requestId === 'unrelated'), 'unrelated native event')
  assert(delayed.jobId === null, 'Must not adopt another request')
  await delayed.cancel()
  const waiting = await snapshot()
  assert(!waiting.cancelled && waiting.cancels.length === before.cancels.length, 'Stop intent must wait for ownership')
  await invoke('bridge_release')
  const delayedResult = await delayed.promise
  const afterDelayed = await snapshot()
  assert(delayedResult.cancelled && delayedResult.jobId === delayed.jobId, 'Delayed stop did not reach owner')
  assert(afterDelayed.jobId === null && afterDelayed.cancels.at(-1) === delayed.jobId, 'Scoped cancellation/slot release failed')
  passed.push('delayed_started')

  const owner = controlled('busy-owner')
  await until(() => acquired.some(event => event.requestId === 'busy-owner'), 'busy owner')
  const blocked = cancellableJob(requestId => api.libraryHealth(requestId), error => cancelErrors.push(error))
  await rejected(blocked.promise, '已有任务在运行')
  await blocked.cancel()
  // A completed old helper must not stop the new task either.
  await delayed.cancel()
  const stillBusy = await snapshot()
  assert(blocked.jobId === null && !stillBusy.cancelled, 'Rejected/old task cancelled owner')
  assert(stillBusy.cancels.length === afterDelayed.cancels.length, 'Unexpected cancellation IPC')
  assert(await api.jobCancel(stillBusy.jobId!), 'Could not release own synthetic task')
  await owner
  passed.push('busy_and_settled')

  await invoke('bridge_lock_writer')
  let previewRequest = ''
  const preview = cancellableJob(requestId => {
    previewRequest = requestId
    return api.nfoExportPreview(['SMPL-001', 'SMPL-002'], 'original', requestId)
  }, error => cancelErrors.push(error))
  const previewRejected = rejected(preview.promise, '取消')
  await until(() => announced.some(event => event.requestId === previewRequest) && preview.jobId !== null, 'real preview started')
  await preview.cancel()
  const cancelledPreview = await snapshot()
  assert(cancelledPreview.cancels.at(-1) === preview.jobId && (cancelledPreview.cancelled || cancelledPreview.jobId === null), 'Native preview did not receive cancellation')
  await invoke('bridge_unlock_writer')
  await previewRejected
  assert((await snapshot()).jobId === null, 'Cancelled preview still owns task slot')
  await invoke('bridge_verify', { journals: 0 })
  passed.push('real_preview_cancel')

  const ready = cancellableJob(requestId => api.nfoExportPreview(['SMPL-001', 'SMPL-002'], 'original', requestId), error => cancelErrors.push(error))
  const prepared = await ready.promise
  assert(prepared.items.length === 2, 'Actual preview did not return both films')
  const apply = cancellableJob(requestId => api.nfoExportApply(prepared.token, prepared.root, requestId), error => cancelErrors.push(error))
  const applied = await apply.promise
  assert(!applied.cancelled && applied.failed.length === 0 && applied.results.length === 2, 'Actual export failed')
  for (const result of applied.results) {
    assert(!result.error && !result.indexError, 'Export reported disk/index error')
    const detail = await api.libraryGet(result.code)
    assert(detail?.meta.nfo_language === 'original', 'Export index is stale')
  }
  await invoke('bridge_verify', { journals: 2, exported: true })
  passed.push('real_export')

  for (const result of [...applied.results].reverse()) {
    const rolled = await api.rollback(result.journalId)
    assert(rolled.failures.length === 0 && !rolled.indexError, 'Actual rollback failed')
    assert((await api.libraryGet(result.code))?.meta.nfo_language === 'current', 'Rollback index is stale')
  }
  await invoke('bridge_verify', { journals: 2 })
  assert((await snapshot()).jobId === null && cancelErrors.length === 0, 'Outstanding task or cancellation error')
  assert((await snapshot()).cancels.every(id => typeof id === 'number'), 'Unscoped cancellation was sent')
  passed.push('real_rollback')
  await runNetworkChecks(passed)
  await runImportUi(passed)
  await runRebuildChecks(passed)
  stopAcquired()
  stopAnnounced()
}

run().then(async () => {
  document.querySelector('#status')!.textContent = '验收通过'
  await invoke('bridge_finish', { passed, error: null })
}).catch(async error => {
  document.querySelector('#status')!.textContent = String(error)
  await invoke('bridge_finish', { passed, error: error instanceof Error ? `${error.message}\n${error.stack ?? ''}` : String(error) })
})
