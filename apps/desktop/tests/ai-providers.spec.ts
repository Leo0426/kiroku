import { expect, test, type Page } from '@playwright/test'
import { overrideApi } from './helpers'

async function openAi(page: Page) {
  await page.goto('/')
  await page.getByRole('navigation', { name: '主导航' }).getByRole('button', { name: '设置', exact: true }).click()
  await page.getByRole('tab', { name: 'AI 汉化', exact: true }).click()
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    if (!localStorage.getItem('kiroku-mock-settings-v2')) localStorage.setItem('kiroku-mock-settings-v2', JSON.stringify({
      library_root: '/mock/library', ai: { enabled: true, api_key: 'legacy-key', model: 'legacy-model',
        base_url: 'https://legacy.example', effort: 'low', title: false, plot: true },
    }))
  })
})

test('migrates legacy settings, saves independent providers, and never falls back after deletion', async ({ page }) => {
  await openAi(page)
  await expect(page.getByLabel('API Key', { exact: true })).toHaveValue('legacy-key')
  await expect(page.getByLabel('模型', { exact: true })).toHaveValue('legacy-model')
  await page.getByLabel('新 Provider 类型').selectOption('openai')
  await page.getByRole('button', { name: '添加 Provider', exact: true }).click()
  await page.getByLabel('Provider 名称').fill('My Gateway')
  await page.getByLabel('API Key', { exact: true }).fill('gateway-key')
  await page.getByLabel('API 地址', { exact: true }).fill('https://gateway.example/v1')
  await page.getByLabel('模型', { exact: true }).fill('gateway-model')
  await page.getByRole('button', { name: /保存更改/ }).click()
  await expect(page.getByText('设置已保存', { exact: true })).toBeVisible()
  await openAi(page)
  await expect(page.getByLabel('Provider 名称')).toHaveValue('My Gateway')
  await page.getByLabel('当前 Provider').selectOption({ label: 'Anthropic' })
  await expect(page.getByLabel('API Key', { exact: true })).toHaveValue('legacy-key')
  await expect(page.getByLabel('模型', { exact: true })).toHaveValue('legacy-model')
  await page.getByRole('button', { name: '删除 Provider', exact: true }).click()
  await expect(page.getByLabel('当前 Provider')).toHaveValue('')
  await expect(page.getByText('请选择用于汉化的 AI Provider', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: /保存更改/ }).click()
  await expect(page.getByRole('alert')).toContainText('请选择用于汉化的 AI Provider')
  await page.getByLabel('当前 Provider').selectOption({ label: 'My Gateway' })
  await page.getByRole('button', { name: /保存更改/ }).click()
  await expect(page.getByRole('button', { name: '已保存', exact: true })).toBeVisible()
  const ai = await page.evaluate(() => JSON.parse(localStorage.getItem('kiroku-mock-settings-v2')!).ai)
  expect(ai.providers).toHaveLength(1)
  expect(ai.providers[0]).toMatchObject({ protocol: 'openai', api_key: 'gateway-key', model: 'gateway-model' })
  expect(ai.api_key).toBeUndefined()
})

test('trial uses unsaved selected provider and discards changed-away-and-back results', async ({ page }) => {
  await overrideApi(page, `window.trialDraft=null; window.finishTrial=null; api.aiTest=async cfg=>{window.trialDraft=cfg;await new Promise(r=>window.finishTrial=r);return {title:'试译结果',plot:null}};`)
  await openAi(page)
  await page.getByLabel('新 Provider 类型').selectOption('custom')
  await page.getByRole('button', { name: '添加 Provider', exact: true }).click()
  await page.getByLabel('API Key', { exact: true }).fill('draft-key')
  await page.getByLabel('API 地址', { exact: true }).fill('https://custom.example/v1')
  await page.getByLabel('模型', { exact: true }).fill('draft-model')
  await page.getByRole('button', { name: '试译', exact: true }).click()
  await expect.poll(() => page.evaluate(() => !!(window as any).finishTrial)).toBe(true)
  const draft = await page.evaluate(() => (window as any).trialDraft)
  expect(draft.providers.find((p: any) => p.id === draft.selected_provider)).toMatchObject({ api_key: 'draft-key', model: 'draft-model', protocol: 'openai' })
  await page.getByLabel('模型', { exact: true }).fill('changed')
  await page.getByLabel('模型', { exact: true }).fill('draft-model')
  await page.evaluate(() => (window as any).finishTrial())
  await expect(page.getByRole('status', { name: 'AI 试译结果' })).toHaveCount(0)
  const saved = await page.evaluate(() => JSON.parse(localStorage.getItem('kiroku-mock-settings-v2')!).ai)
  expect(saved.api_key).toBe('legacy-key')
  await page.getByRole('button', { name: '试译', exact: true }).click()
  await page.evaluate(() => (window as any).finishTrial())
  await expect(page.getByRole('status', { name: 'AI 试译结果' })).toContainText('试译结果')
})
