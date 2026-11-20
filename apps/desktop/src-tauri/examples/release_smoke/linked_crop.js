const invoke = (command, args = {}) => window.__TAURI_INTERNALS__.invoke(command, args).catch(error => {
  throw Error(command+': '+String(error))
})
const report = message => invoke('release_report', {message}).catch(() => {})
window.addEventListener('error', event => {
  if (event.target instanceof HTMLImageElement && event.target.src.includes('library')) return
  report('ERROR: '+(event.message || event.target?.src || 'resource load'))
}, true)
window.addEventListener('unhandledrejection', event => report('REJECTION: '+String(event.reason)))

const code = 'TEST-105'
function assert(value, message) {
  if (!value) throw Error(message)
}
async function visiblePoster(width) {
  for (let attempt = 0; attempt < 100; attempt++) {
    const image = document.querySelector('button.pcard[aria-label^="TEST-105 "] .poster img')
    if (image) {
      image.scrollIntoView({block:'center'})
      image.loading = 'eager'
      if (image.naturalWidth === width && image.classList.contains('ready')) return
    }
    await new Promise(resolve => setTimeout(resolve, 60))
  }
  throw Error('poster wall did not decode the '+width+' pixel revision')
}
async function inspectDisk(initial, width, crop) {
  const disk = await invoke('release_linked_files')
  assert(disk.same && disk.initial === initial && disk.dimensions[0] === width, 'Plex alias and movie poster diverged')
  assert(disk.originalOk && disk.videoOk && disk.oldDirectoryAbsent, 'crop modified the original, video, or old directory')
  assert((disk.crop?.x ?? null) === (crop?.x ?? null), 'on-disk crop position changed unexpectedly')
}
async function until(label, read) {
  for (let n = 0; n < 150; n++) {
    const result = await read()
    if (result) return result
    await new Promise(resolve => setTimeout(resolve, 40))
  }
  throw Error(label)
}
async function openDuringScan() {
  document.querySelector('button.pcard[aria-label^="TEST-105 "]').click()
  const openEditor = await until('missing editor action', () => document.querySelector('button[aria-label="重新裁剪海报"]'))
  await until('fixture could not claim the released scan slot', () => invoke('release_scan_fixture', {hold:true})
    .then(() => true, error => {if (String(error).includes('已有任务在运行')) return false; throw error}))
  openEditor.click()
  const editor = () => document.querySelector('[role="dialog"][aria-label="裁剪海报"]')
  await until('editor did not wait for pending scan', () => editor()?.textContent.includes('正在检查待整理文件'))
  await invoke('release_scan_fixture', {hold:false})
  await until('editor did not automatically reopen the original', () =>
    editor()?.querySelector('img[alt="裁剪原图"]')?.naturalWidth === 128)
  editor().querySelector('button[aria-label="关闭裁剪"]').click()
  document.querySelector('button[aria-label="关闭详情"]').click()
}
async function run() {
  await visiblePoster(90)
  await openDuringScan()
  const before = await invoke('library_get', {code})
  const edit = await invoke('artwork_open', {code, requestId:'fixture-open'})
  assert(edit.width === 128 && edit.height === 128 && edit.image.length > 100, 'missing source original')
  const crop = {x: 23, y: 18, width: 56, height: 80}
  const preview = await invoke('artwork_preview', {code, root:edit.root, token:edit.token, crop, requestId:'fixture-preview'})
  const revision = preview.revision
  assert(preview.image.length > 100 && revision.code === code, 'missing cropped preview')
  const save = await invoke('revision_save', {code, root:revision.root, token:revision.token})
  assert(save.journalId && !save.error && !save.indexError, 'crop save or index update failed')
  await inspectDisk(false, 56, crop)
  await visiblePoster(56)
  const updated = await invoke('library_list', {query:{}})
  assert(updated.total === 1 && updated.items[0].code === code && updated.items[0].posterVersion !== before.posterVersion, 'index lost the relocated movie or crop revision')
  const history = await invoke('history_list', {requestId:'fixture-history-list', limit:20})
  assert(history.items.some(row => row.id === save.journalId && row.codes.includes(code)), 'linked crop is missing from history')
  const journal = await invoke('history_detail', {requestId:'fixture-history', journalId:save.journalId, offset:0})
  assert(journal.id === save.journalId && journal.total > 0, 'crop history is not accessible')
  const undo = await invoke('rollback', {journalId:save.journalId})
  assert(!undo.indexError && !undo.failures.length, 'undo of linked crop failed')
  await inspectDisk(true, 90, null)
  await visiblePoster(90)
  const restored = await invoke('library_get', {code})
  assert(restored.posterVersion === before.posterVersion, 'undo did not restore the original poster revision')
  const journalAfter = await invoke('history_detail', {requestId:'fixture-undo-history', journalId:save.journalId, offset:0})
  assert(journalAfter.status !== journal.status, 'undo did not update history')
  const again = await invoke('artwork_open', {code, requestId:'fixture-reopen'})
  assert(again.width === 128 && again.height === 128 && !again.crop, 'original cannot be reopened after undo')
  report('READY: production linked crop roundtrip')
}
document.addEventListener('DOMContentLoaded', () => run().catch(error => report('ERROR: '+String(error))))
