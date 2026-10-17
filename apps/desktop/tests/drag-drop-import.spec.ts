import { expect, test, type Page } from '@playwright/test'
import { overrideApi, seedLibrary } from './helpers'

test.beforeEach(async ({ page }) => seedLibrary(page))

/** 浏览器预览没有原生拖放路径,mock 通过窗口事件模拟一次原生拖放。 */
async function drag(page: Page, type: 'enter' | 'over' | 'leave' | 'drop', paths: string[] = []) {
  await page.evaluate(([type, paths]) => {
    window.dispatchEvent(new CustomEvent('kiroku-mock-drag', { detail: { type, paths } }))
  }, [type, paths] as const)
}

const sourceList = (page: Page) => page.getByRole('region', { name: '待整理的文件夹' })

test('dropping folders on any page opens import with them added to the source list', async ({ page }) => {
  await page.goto('/')
  await expect(page.getByRole('button', { name: '媒体库', exact: true })).toBeVisible()
  await drag(page, 'enter', ['/mock/dropped-a', '/mock/dropped-b'])
  await drag(page, 'drop', ['/mock/dropped-a', '/mock/dropped-b'])
  await expect(sourceList(page)).toContainText('/mock/dropped-a')
  await expect(sourceList(page)).toContainText('/mock/dropped-b')
})

test('only folders outside the library are added and the rest are explained', async ({ page }) => {
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await drag(page, 'drop', ['/mock/new-a', '/mock/new-a/', '/mock/clip.txt', '/mock/missing-x', '/mock/library/sub'])
  await expect(page.getByRole('button', { name: '移除 /mock/new-a', exact: true })).toHaveCount(1)
  await expect(sourceList(page)).not.toContainText('clip.txt')
  await expect(sourceList(page)).not.toContainText('/mock/library/sub')
  const alert = page.getByRole('alert')
  await expect(alert).toContainText('不是文件夹或视频')
  await expect(alert).toContainText('clip.txt')
  await expect(alert).toContainText('missing-x')
  await expect(alert).toContainText('媒体库')
})

test('dragging shows a drop hint, and an import in progress is left untouched', async ({ page }) => {
  await overrideApi(page, `
    const getSettings = api.getSettings;
    api.getSettings = async () => ({ ...(await getSettings()), source_dirs: ['/mock/source'] });
  `)
  await page.goto('/')
  await drag(page, 'enter', ['/mock/new-a'])
  await expect(page.getByText('松开以添加待整理文件夹')).toBeVisible()
  await drag(page, 'leave')
  await expect(page.getByText('松开以添加待整理文件夹')).toHaveCount(0)

  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await expect(page.getByRole('button', { name: /查找影片信息/ })).toBeEnabled()
  await drag(page, 'enter', ['/mock/late'])
  await expect(page.getByText(/当前这批还没整理完/)).toBeVisible()
  await drag(page, 'drop', ['/mock/late'])
  await expect(page.getByText(/当前这批还没整理完/)).toHaveCount(0)
  await expect(page.getByRole('button', { name: /查找影片信息/ })).toBeEnabled()
  await page.getByRole('button', { name: '重新选择', exact: true }).click()
  await expect(sourceList(page)).toContainText('/mock/source')
  await expect(sourceList(page)).not.toContainText('/mock/late')
})

test('a dropped video file is imported on its own and not remembered as a source folder', async ({ page }) => {
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  await drag(page, 'drop', ['/mock/dl/SMPL-421.mp4', '/mock/new-a', '/mock/notes.txt'])
  await expect(sourceList(page)).toContainText('/mock/dl/SMPL-421.mp4')
  await expect(sourceList(page)).toContainText('单个文件')
  await expect(page.getByRole('alert')).toContainText('notes.txt')
  await page.getByRole('button', { name: '移除 /mock/new-a', exact: true }).click()
  await page.getByRole('button', { name: '开始扫描', exact: true }).click()
  await expect(page.getByRole('button', { name: /查找影片信息（1 部）/ })).toBeEnabled()
  const saved = await page.evaluate(() => JSON.parse(localStorage.getItem('kiroku-mock-settings-v2')!))
  expect(saved.source_dirs ?? []).not.toContain('/mock/dl/SMPL-421.mp4')
})
