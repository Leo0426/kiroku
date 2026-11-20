import { expect, test } from '@playwright/test'

test('scans every chosen folder and classifies explicit video paths once', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('kiroku-mock-settings-v2', JSON.stringify({
    library_root: '/mock/library', source_dirs: ['/mock/first', '/mock/second'],
  })))
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  const known = 'C:\\Videos\\DEMO-537.mkv'
  const unknown = '/mock/extra/weekend recording.mov'
  const trailer = '/mock/extra/DEMO-914-trailer.mkv'
  page.once('dialog', dialog => dialog.accept(['/mock/first/SMPL-421.mp4', known, unknown, trailer].join(' | ')))
  await page.getByRole('button', { name: '添加视频文件' }).click()
  await expect(page.getByRole('button', { name: '移除 ' + known })).toBeVisible()
  await page.getByRole('button', { name: '开始扫描' }).click()
  await expect(page.getByRole('button', { name: /查找影片信息/ })).toBeEnabled()
  await expect(page.getByRole('region', { name: '需要确认的文件' }).getByTitle(unknown)).toBeVisible()
  await page.getByText(/查看已识别文件/).click()
  const scanned = page.getByRole('region', { name: '扫描结果' })
  await expect(scanned.getByTitle('/mock/first/SMPL-421.mp4', { exact: true })).toHaveCount(1)
  await expect(scanned.getByTitle('/mock/second/SMPL-421.mp4', { exact: true })).toHaveCount(1)
  await expect(scanned.getByTitle(known, { exact: true })).toBeVisible()
  await expect(scanned.getByRole('button', { name: '修改番号 DEMO-537', exact: true })).toBeVisible()
  await page.getByText(/已跳过 \d+ 个文件/).click()
  await expect(page.getByTitle(trailer, { exact: true })).toBeVisible()
})

test('organizing a picked video does not mark its enclosing directory for cleanup', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('kiroku-mock-settings-v2', JSON.stringify({
    library_root: '/mock/library', source_dirs: [],
  })))
  await page.goto('/')
  await page.getByRole('button', { name: '导入整理', exact: true }).click()
  page.once('dialog', dialog => dialog.accept('/Volumes/Downloads/新片/companion-import/DEMO-871.mp4'))
  await page.getByRole('button', { name: '添加视频文件' }).click()
  await page.getByRole('button', { name: '开始扫描' }).click()
  await page.getByRole('button', { name: '预览整理', exact: true }).click()
  await page.getByRole('button', { name: '确认整理（1 个文件）' }).click()
  await expect(page.getByRole('heading', { name: '整理完成' })).toBeVisible()
  await expect(page.getByLabel('本批整理结果').getByText(/原文件夹移到废纸篓/)).toHaveCount(0)
})
