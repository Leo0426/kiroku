import { expect, test } from '@playwright/test'
import { overrideApi } from './helpers'

test('首次接入已有媒体库时等待索引重建，完成后显示影片', async ({ page }) => {
  await overrideApi(page, `
    api.pickDirectory = async () => '/mock/existing'
    const list = api.libraryList
    let indexed = false
    api.libraryList = async query => indexed ? list(query) : { total: 0, items: [] }
    const { emitMock } = await import('/src/lib/events.ts')
    api.libraryRebuild = async () => {
      window.finishIndex = () => {
        indexed = true
        emitMock('library:rebuilt', { jobId: 12, root: '/mock/existing', count: 3, error: null, cancelled: false })
      }
      return 12
    }
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '选择其他文件夹' }).click()
  await expect(page.getByRole('heading', { name: '正在从已有影片建立索引' })).toBeVisible()
  await expect(page.getByText('媒体库还是空的')).toHaveCount(0)
  await expect(page.getByText('媒体库已就绪')).toHaveCount(0)
  await page.evaluate(() => (window as unknown as { finishIndex(): void }).finishIndex())
  await expect(page.locator('button[data-card-index]').first()).toBeVisible()
  await expect(page.getByText('索引已重建:3 部影片')).toBeVisible()
})

test('首次重建启动失败后能从海报墙重试，零条结果才展示空库', async ({ page }) => {
  await overrideApi(page, `
    api.pickDirectory = async () => '/mock/existing'
    api.libraryList = async () => ({ total: 0, items: [] })
    const { emitMock } = await import('/src/lib/events.ts')
    let first = true
    api.libraryRebuild = async () => {
      if (first) { first = false; throw new Error('目录暂时不可读') }
      emitMock('library:rebuilt', { jobId: 13, root: '/mock/existing', count: 0, error: null, cancelled: false })
      return 13
    }
  `)
  await page.goto('/')
  await page.getByRole('button', { name: '选择其他文件夹' }).click()
  await expect(page.getByRole('heading', { name: '媒体库索引尚未更新' })).toBeVisible()
  await expect(page.getByText('媒体库还是空的')).toHaveCount(0)
  await page.getByRole('button', { name: '重试重建索引' }).click()
  await expect(page.getByText('索引已重建:0 部影片')).toBeVisible()
  await expect(page.getByRole('heading', { name: '媒体库还是空的' })).toBeVisible()
  await expect(page.getByRole('button', { name: '导入影片' })).toBeVisible()
})

test('选错文件夹后可在提醒中重新选择，也可用 Esc 返回首次设置', async ({ page }) => {
  const choices = ['/mock/Downloads', '/mock/Downloads', '/mock/library']
  page.on('dialog', dialog => dialog.accept(choices.shift()))
  await page.goto('/')
  const choose = page.getByRole('button', { name: '选择其他文件夹' })
  await choose.click()
  const warning = page.getByRole('alertdialog', { name: '这个文件夹里有还没整理的视频' })
  await expect(warning).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(warning).not.toBeVisible()
  await choose.click()
  const reselect = warning.getByRole('button', { name: '重新选择' })
  await expect(reselect).toBeFocused()
  await reselect.press('Enter')
  await expect(page.getByRole('heading', { name: '媒体库', exact: true })).toBeVisible()
  await expect(page.getByTitle('/mock/library', { exact: true })).toBeVisible()
})

test('选择保留原始视频目录作为媒体库时使用该目录', async ({ page }) => {
  page.once('dialog', dialog => dialog.accept('/mock/Downloads'))
  await page.goto('/')
  await page.getByRole('button', { name: '选择其他文件夹' }).click()
  await page.getByRole('alertdialog').getByRole('button', { name: '仍然使用它' }).click()
  await expect(page.getByTitle('/mock/Downloads', { exact: true })).toBeVisible()
})
