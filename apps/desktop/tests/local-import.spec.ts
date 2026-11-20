import { expect, test } from '@playwright/test'
import { overrideApi } from './helpers'

test('all-local imports can preview directly from the scan and refresh in place if asked', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('kiroku-mock-settings-v2', JSON.stringify({
      library_root: '/mock/library',
      source_dirs: ['/mock/companion-import'],
      routes: { censored: [] },
    }))
  })
  await overrideApi(page, `const scrape=api.scrapeStart; window.scrapeCalls=[]; api.scrapeStart=(...args)=>{window.scrapeCalls.push(args);return scrape(...args)}`)
  await page.goto('/')
  await page.getByRole('navigation', { name: '主导航' }).getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  const ready = page.getByRole('region', { name: '本地资料', exact: true })
  await expect(ready).toContainText('本地配套：NFO、海报、背景图')
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  await expect(page.getByRole('region', { name: '整理计划', exact: true })).toBeVisible()
  expect(await page.evaluate(() => (window as any).scrapeCalls)).toHaveLength(0)
  await page.getByRole('button', { name: '上一步', exact: true }).click()
  const first = ready.getByRole('article').first()
  await first.getByRole('button', { name: /重新抓取/ }).click()
  await expect(first.getByText(/沿用现有资料/)).toBeVisible()
  await expect(page.getByRole('button', { name: '预览整理', exact: true })).toBeEnabled()
  const calls = await page.evaluate(() => (window as any).scrapeCalls)
  expect(calls).toHaveLength(1)
  expect(calls[0].slice(1)).toEqual([false, true])
})

for (const { filename, resolves } of [
  { filename: 'MOVX-679-C.mp4', resolves: true },
  { filename: 'WXYZ-377.mp4', resolves: false },
]) {
  test(`mixed import only looks up the missing film and ${resolves ? 'advances when all are ready' : 'offers the ready films on failure'}`, async ({ page }) => {
    await page.addInitScript(({ filename }) => {
      const source = '/mock/companion-mixed'
      localStorage.setItem('kiroku-mock-settings-v2', JSON.stringify({
        library_root: '/mock/library',
        source_dirs: [`${source}/SMPL-421.mp4`, `${source}/${filename}`],
      }))
    }, { filename })
    await overrideApi(page, `const scrape=api.scrapeStart; window.scrapeCalls=[]; api.scrapeStart=(...args)=>{window.scrapeCalls.push(args);return scrape(...args)}`)
    await page.goto('/')
    await page.getByRole('navigation', { name: '主导航' }).getByRole('button', { name: '导入整理', exact: true }).click()
    await page.getByRole('button', { name: '开始扫描', exact: true }).click()
    const ready = page.getByRole('region', { name: '本地资料', exact: true })
    await expect(ready).toContainText('已就绪 1 部 · 待补齐 1 部')

    await page.getByRole('button', { name: '仅预览就绪影片', exact: true }).click()
    const plan = page.getByRole('region', { name: '整理计划', exact: true })
    await expect(plan.getByRole('checkbox')).toHaveCount(1)
    expect(await page.evaluate(() => (window as any).scrapeCalls)).toHaveLength(0)
    await page.getByRole('button', { name: '上一步', exact: true }).click()
    await page.getByRole('button', { name: '补齐 1 部并预览', exact: true }).click()
    if (resolves) {
      await expect(plan.getByRole('checkbox')).toHaveCount(2)
    } else {
      await expect(page.getByRole('region', { name: '需要处理的影片' })).toContainText('WXYZ-377')
      await expect(plan).toHaveCount(0)
      await page.getByRole('button', { name: '预览整理', exact: true }).click()
      await expect(plan.getByRole('checkbox')).toHaveCount(1)
    }
    expect(await page.evaluate(() => (window as any).scrapeCalls.map((args: unknown[]) => args[0]))).toEqual([[1]])
  })
}
