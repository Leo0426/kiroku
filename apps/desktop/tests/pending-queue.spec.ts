import { expect, test } from '@playwright/test'
import { seedLibrary } from './helpers'

test('cancelled queued scans settle while the dispatched read remains blocked', async ({ page }) => {
  await seedLibrary(page)
  await page.goto('/')
  await page.evaluate(async () => {
    const { pendingRead, cancelPendingReads } = await import('/src/lib/pending.ts')
    const state = window as unknown as { releaseScan: () => void; queuedCancelled: number; queuedStarted: number; idle: boolean; activeFinished: boolean }
    state.queuedCancelled = 0; state.queuedStarted = 0; state.idle = false; state.activeFinished = false
    const active = pendingRead(() => new Promise<void>(resolve => { state.releaseScan = resolve }))
    active.promise.then(() => { state.activeFinished = true })
    await Promise.resolve()
    for (let i = 0; i < 100; i++) {
      const queued = pendingRead(async () => { state.queuedStarted++ })
      queued.promise.catch(() => { state.queuedCancelled++ })
      queued.cancel()
    }
    void cancelPendingReads().then(() => { state.idle = true })
  })
  await expect.poll(() => page.evaluate(() => (window as unknown as { queuedCancelled: number }).queuedCancelled)).toBe(100)
  expect(await page.evaluate(() => (window as unknown as { queuedStarted: number }).queuedStarted)).toBe(0)
  expect(await page.evaluate(() => (window as unknown as { idle: boolean }).idle)).toBe(false)
  await page.evaluate(() => (window as unknown as { releaseScan: () => void }).releaseScan())
  await expect.poll(() => page.evaluate(() => (window as unknown as { idle: boolean }).idle)).toBe(true)
  expect(await page.evaluate(() => (window as unknown as { activeFinished: boolean }).activeFinished)).toBe(true)
})
