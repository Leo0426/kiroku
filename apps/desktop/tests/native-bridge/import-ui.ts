import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { mount, tick, unmount } from 'svelte'
import Import from '../../src/views/Import.svelte'
import '../../src/styles/base.css'
import { api } from '../../src/lib/api'
import { flow } from '../../src/lib/importflow.svelte'
import { app } from '../../src/lib/state.svelte'
import type { Settings } from '../../src/lib/types'

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message)
}
async function until(predicate: () => boolean, message: string) {
  const deadline = Date.now() + 5000
  while (!predicate()) {
    if (Date.now() >= deadline) throw new Error(`Timed out: ${message}; import error: ${flow.error}`)
    await new Promise(resolve => setTimeout(resolve, 10))
  }
}
const job = () => invoke<{ jobId: number | null, cancels: number[] }>('bridge_snapshot')
const network = () => invoke<{ requests: string[], holding: boolean }>('bridge_network_snapshot')

export async function runImportUi(passed: string[]) {
  const fixture = await invoke<{ settings: Settings, inputRoot: string, base: string }>('bridge_network_prepare', { scanFromUi: true })
  flow.reset()
  flow.dirs = [fixture.inputRoot]
  app.settings = fixture.settings
  const host = document.createElement('main')
  host.style.cssText = 'height:100vh;overflow:auto'
  document.body.append(host)
  const component = mount(Import, { target: host })
  let waiting: number | null = null
  const stopWaiting = await listen<{ jobId: number }>('bridge:network_waiting', event => { waiting = event.payload.jobId })
  const label = (element: Element) => (element.getAttribute('aria-label') ?? element.textContent ?? '').replace(/\s+/g, ' ').trim()
  const buttons = (name: RegExp) => [...host.querySelectorAll<HTMLButtonElement>('button')].filter(button => name.test(label(button)))
  const has = (selector: string, text: string) => [...host.querySelectorAll(selector)].some(element => label(element) === text)
  async function click(name: RegExp) {
    await until(() => buttons(name).length === 1, `one button ${name}`)
    const button = buttons(name)[0]
    assert(!button.disabled && button.getClientRects().length > 0, `Unavailable button: ${label(button)}`)
    button.scrollIntoView({ block: 'center' })
    await new Promise<void>((resolve, reject) => {
      const timeout = setTimeout(() => {
        cancelAnimationFrame(frame)
        reject(new Error('No WebKit render frame; the macOS session may be locked or the window hidden'))
      }, 5000)
      const frame = requestAnimationFrame(() => { clearTimeout(timeout); resolve() })
    })
    button.click()
    await tick()
  }
  try {
    await tick()
    assert(has('h1', '导入整理') && has('li[aria-current="step"] .lbl', '选择文件夹'), 'Import view did not mount at source step')
    await click(/^开始扫描$/)
    await until(() => !flow.scanRunning && flow.scanSummary !== null, 'real scan completion')
    await tick()
    assert([...flow.items].length === 3 && flow.scanSummary?.complete && !flow.error, 'UI scan did not discover fixture videos')
    assert(has('li[aria-current="step"] .lbl', '识别文件'), 'Scan stepper not updated')
    const [first, second, third] = flow.uniqueCodes
    // Real directory iteration has no lexical-order guarantee. Hold the actual
    // second item without rearranging the scanned production session or UI.
    await invoke('bridge_network_hold', { code: second })
    await click(/^查找影片信息（3 部）$/)
    await until(() => waiting !== null && flow.statuses[first]?.kind === 'found', 'held second request')
    await tick()
    assert(has('h2', '正在查找影片信息'), 'Busy scrape heading missing')
    assert(buttons(/^预览整理$/)[0]?.disabled && buttons(/^上一步$/)[0]?.disabled, 'Busy view permits conflicting navigation or preview')
    const owner = waiting
    await click(/^停止$/)
    await until(() => !flow.scrape.running, 'stop button completion')
    await tick()
    const stopped = await job()
    assert(stopped.jobId === null && stopped.cancels.at(-1) === owner, 'UI stop did not release its own job')
    assert(has('h2', '已停止') && buttons(/^停止$/).length === 0, 'Stopped state not rendered')
    const stoppedPreview = buttons(/^预览整理$/)
    assert(stoppedPreview.length === 1 && stoppedPreview[0].disabled === false, 'Completed first item cannot be previewed after stop')
    assert(buttons(/^重试未完成（2 部）$/).length === 1, `Retry count wrong: buttons=${buttons(/重试/).map(label)}; codes=${flow.uniqueCodes}; statuses=${JSON.stringify(flow.statuses)}`)
    const partial = await network()
    assert(partial.holding && partial.requests.join(',') === `/${first},/${second}`, 'UI stop waited for EOF or requested the third item')
    await invoke('bridge_network_release')

    // Use the actual detail-page form to recover the cancelled second item.
    await click(new RegExp(`^为 ${second} 指定详情页$`))
    const input = host.querySelector<HTMLInputElement>(`input[aria-label="${second} 详情页地址"]`)!
    assert(input && buttons(/^用详情页刮削$/)[0]?.disabled, 'Empty detail URL is not gated')
    input.value = `${fixture.base}/${second}`
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await tick()
    await click(/^用详情页刮削$/)
    await until(() => !flow.scrape.running && flow.statuses[second]?.kind === 'found', 'detail form result')
    await tick()
    assert(buttons(/^重试未完成（1 部）$/).length === 1, 'Successful detail result was not removed from retry list')
    await click(/^重试未完成（1 部）$/)
    await until(() => !flow.scrape.running && flow.found.length === 3, 'remaining film retry')
    await tick()
    assert(has('h2', '影片信息查询完成') && !host.querySelector('[aria-label="需要处理的影片"]'), 'Completed scrape UI is stale')
    assert((await network()).requests.join(',') === `/${first},/${second},/${second},/${third}`, 'UI recovery requested wrong films')
    const foundSummary = [...host.querySelectorAll<HTMLElement>('summary')].find(element => label(element) === '查看已找到的影片 · 3 部')
    assert(foundSummary, 'Found result disclosure missing')
    foundSummary.click()
    await until(() => ['NET-101', 'NET-102', 'NET-103'].every(code => host.querySelector('[aria-label="已找到的影片"]')?.textContent?.includes(`Neutral ${code}`)), 'rendered result titles')

    await click(/^预览整理$/)
    await until(() => flow.step === 'plan' && !flow.planning, 'real organize preview')
    await tick()
    assert(host.querySelectorAll('[aria-label="整理计划"] input:checked').length === 3 && flow.conflictCount === 0, 'Plan is incomplete or conflicted')
    assert(host.querySelector('[aria-label="文件处理方式"]')?.textContent?.includes('复制视频和字幕'), 'UI did not disclose copy mode')
    await click(/^确认整理（3 个文件）$/)
    await until(() => flow.step === 'done' && !flow.apply.running, 'real organize completion')
    await tick()
    const applied = flow.applyDone
    assert(applied && applied.applied === 3 && applied.failed.length === 0 && !applied.cancelled && !applied.indexError && applied.cleaned.length === 0, 'Import apply failed or cleaned sources')
    assert(has('h2', '整理完成') && buttons(/^撤销本批$/).length === 1, 'Done UI did not expose rollback')
    for (const code of ['NET-101', 'NET-102', 'NET-103']) {
      const detail = await api.libraryGet(code)
      assert(detail?.meta.title === `Neutral ${code}` && detail.meta.provenance?.title === 'javbus', `Imported index is wrong: ${code}`)
    }
    await invoke('bridge_import_verify', { journalId: applied.journalId, rolledBack: false })

    await click(/^撤销本批$/)
    assert(buttons(/^确认撤销$/).length === 1, 'Undo confirmation missing')
    await click(/^确认撤销$/)
    await until(() => host.textContent?.includes('本批已撤销') === true, 'undo success in UI')
    assert(buttons(/^撤销本批$/).length === 0 && (await job()).jobId === null, 'Undo did not settle')
    for (const code of ['NET-101', 'NET-102', 'NET-103']) assert(await api.libraryGet(code) === null, `Rolled-back film remains indexed: ${code}`)
    for (const code of ['SMPL-001', 'SMPL-002']) {
      const original = await api.libraryGet(code)
      assert(original?.meta.title === 'Localized sample' && original.meta.nfo_language === 'current', `Existing index changed: ${code}`)
    }
    await invoke('bridge_import_verify', { journalId: applied.journalId, rolledBack: true })
    assert(!flow.error && !host.querySelector('[role="alert"]'), 'Import view retained an error')
    await click(/^再来一批$/)
    assert(flow.step === 'source' && !flow.busy && flow.items.length === 0, 'New batch did not reset the component')
    passed.push('import_component_roundtrip')
  } finally {
    stopWaiting()
    await unmount(component)
    host.remove()
  }
}
