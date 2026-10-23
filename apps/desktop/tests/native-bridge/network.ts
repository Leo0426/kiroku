import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { flow } from '../../src/lib/importflow.svelte'
import { app } from '../../src/lib/state.svelte'
import type { ScanItem, Settings } from '../../src/lib/types'

type Fixture = { items: ScanItem[], settings: Settings, base: string }
type Network = { holding: boolean, requests: string[], metas: Record<string, { title: string, source: string }> }
type Job = { jobId: number | null, cancelled: boolean, cancels: number[] }
type Waiting = { jobId: number, path: string }

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message)
}
async function until(predicate: () => boolean, message: string) {
  const deadline = Date.now() + 5000
  while (!predicate()) {
    if (Date.now() >= deadline) throw new Error(`Timed out: ${message}`)
    await new Promise(resolve => setTimeout(resolve, 10))
  }
}
const network = () => invoke<Network>('bridge_network_snapshot')
const job = () => invoke<Job>('bridge_snapshot')
function ordered(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(ordered)
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).sort(([a], [b]) => a.localeCompare(b)).map(([key, item]) => [key, ordered(item)]))
  return value
}
function equal(actual: unknown, expected: unknown, message: string) {
  assert(JSON.stringify(ordered(actual)) === JSON.stringify(ordered(expected)), `${message}: ${JSON.stringify(actual)}`)
}
function checkSummary(cancelled: boolean, found: number, jobId?: number) {
  const summary = flow.scrapeSummary
  assert(!flow.error && summary !== null, 'Missing summary or import error')
  assert(summary.cancelled === cancelled && summary.found === found && summary.failed === 0 && summary.notFound === 0, `Wrong summary: ${JSON.stringify(summary)}`)
  if (jobId !== undefined) assert(summary.jobId === jobId, 'Summary belongs to another task')
}
async function prepare() {
  const fixture = await invoke<Fixture>('bridge_network_prepare')
  assert(!flow.busy, 'Prior import flow must have finished')
  flow.reset()
  flow.items = fixture.items
  flow.step = 'scan'
  app.settings = fixture.settings
  return fixture
}

export async function runNetworkChecks(passed: string[]) {
  const waiting: Waiting[] = []
  const unlisten = await listen<Waiting>('bridge:network_waiting', event => waiting.push(event.payload))
  await flow.init()
  try {
    await prepare()
    await flow.startScrape()
    await until(() => waiting.length === 1 && flow.statuses['NET-101']?.kind === 'found', 'first result and held second response')
    const owner = waiting[0].jobId
    assert(flow.scrape.running && !flow.error, 'Import flow must be running before stop')
    assert((await job()).jobId === owner, 'Network wait belongs to the active scrape')
    await flow.cancel()
    await until(() => !flow.scrape.running, 'cancelled batch done through real event bridge')
    const stopped = await job()
    assert(stopped.jobId === null && stopped.cancels.at(-1) === owner, 'Stopped batch did not release its task slot or used wrong ID')
    checkSummary(true, 1, owner)
    equal(flow.found.map(item => [item.code, item.preview.title, item.preview.sources]), [['NET-101', 'Neutral NET-101', ['javbus']]], 'Keep completed frontend result')
    assert(flow.statuses['NET-102']?.kind === 'cancelled' && flow.statuses['NET-103']?.kind === 'pending', 'Stopped/unstarted item states are wrong')
    const partial = await network()
    assert(partial.holding, 'Cancellation waited for the server to end its partial response')
    equal(partial.requests, ['/NET-101', '/NET-102'], 'Unstarted item was requested')
    equal(partial.metas, { 'NET-101': { title: 'Neutral NET-101', source: 'javbus' } }, 'Backend lost completed metadata')
    await invoke('bridge_network_release')

    await flow.startScrape(true)
    await until(() => !flow.scrape.running, 'retry remaining films')
    checkSummary(false, 2)
    equal(flow.found.map(item => [item.code, item.preview.title, item.preview.sources]), [
      ['NET-101', 'Neutral NET-101', ['javbus']], ['NET-102', 'Neutral NET-102', ['javbus']], ['NET-103', 'Neutral NET-103', ['javbus']],
    ], 'Frontend did not merge retry results')
    const retried = await network()
    equal(retried.requests, ['/NET-101', '/NET-102', '/NET-102', '/NET-103'], 'Retry must request only incomplete films')
    equal(retried.metas, Object.fromEntries(['NET-101', 'NET-102', 'NET-103'].map(code => [code, { title: `Neutral ${code}`, source: 'javbus' }])), 'Backend retry metadata is wrong')
    assert((await job()).jobId === null && !flow.busy, 'Retry left a task active')
    await invoke('bridge_verify', { journals: 2 })
    passed.push('network_batch_cancel_retry')

    const fixture = await prepare()
    const url = `${fixture.base}/NET-102`
    await flow.scrapeUrl('NET-102', url)
    await until(() => waiting.length === 2, 'detail page response held')
    const detailOwner = waiting[1].jobId
    assert((await job()).jobId === detailOwner && flow.scrape.running, 'Detail scrape did not own the active slot')
    await flow.cancel()
    await until(() => !flow.scrape.running, 'cancelled detail done')
    const detailJob = await job()
    assert(detailJob.jobId === null && detailJob.cancels.at(-1) === detailOwner, 'Detail cancellation was not scoped/released')
    checkSummary(true, 0, detailOwner)
    const detailStopped = await network()
    assert(detailStopped.holding, 'Detail stop waited for EOF')
    equal(flow.found, [], 'Cancelled detail published a frontend result')
    equal(detailStopped.metas, {}, 'Cancelled detail published metadata')
    equal(detailStopped.requests, ['/NET-102'], 'Unexpected detail requests')
    await invoke('bridge_network_release')
    await flow.scrapeUrl('NET-102', url)
    await until(() => !flow.scrape.running, 'detail retry success')
    checkSummary(false, 1)
    equal(flow.found.map(item => [item.code, item.preview.title, item.preview.sources]), [['NET-102', 'Neutral NET-102', ['javbus']]], 'Detail result missing from frontend')
    const detailRetried = await network()
    equal(detailRetried.requests, ['/NET-102', '/NET-102'], 'Unexpected detail retry requests')
    equal(detailRetried.metas, { 'NET-102': { title: 'Neutral NET-102', source: 'javbus' } }, 'Detail result missing from backend')
    assert((await job()).jobId === null && !flow.busy && (await job()).cancels.every(id => typeof id === 'number'), 'Detail retry left an active job/unscoped cancellation')
    await invoke('bridge_verify', { journals: 2 })
    passed.push('network_detail_cancel_retry')
  } finally {
    unlisten()
  }
}
