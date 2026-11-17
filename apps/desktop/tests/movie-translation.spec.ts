import { expect, test } from '@playwright/test'
import { seedLibrary, overrideApi } from './helpers'

async function setup(page: import('@playwright/test').Page) {
  await seedLibrary(page)
  await overrideApi(page, `
    const originalGet = api.libraryGet.bind(api);
    api.libraryGet = async (...args) => {
      const d = await originalGet(...args);
      if (d) d.meta.text_versions = {original:{title:'海辺の旅行',plot:'友人と海辺の町を歩く。'},localized:{}};
      return d;
    };
    const originalSettings = api.getSettings.bind(api);
    api.getSettings = async () => {const s=await originalSettings();s.ai.enabled=true;s.ai.providers[0].api_key="test-key";await api.saveSettings(s);return s};
  `)
  await page.goto('/')
  await page.locator('button.pcard').first().click()
  return page.getByRole('region', {name:'单片翻译',exact:true})
}

test('single movie translation shows progress, previews, then saves explicitly', async ({page}) => {
  const section = await setup(page)
  await section.getByRole('button',{name:'翻译此影片'}).click()
  await expect(section.getByRole('progressbar',{name:'翻译进度'})).toBeVisible()
  await expect(section.getByRole('button',{name:'正在翻译…'})).toBeDisabled()
  await expect(section.getByLabel('翻译预览')).toBeVisible()
  await section.getByRole('button',{name:'保存译文'}).click()
  await expect(page.getByText('译文已保存', {exact:true})).toBeVisible()
})

test('cancelled translation never presents a save action', async ({page}) => {
  const section = await setup(page)
  await section.getByRole('button',{name:'翻译此影片'}).click()
  await section.getByRole('button',{name:'取消翻译'}).click()
  await expect(section.getByText('翻译已取消')).toBeVisible()
  await expect(section.getByRole('button',{name:'保存译文'})).toHaveCount(0)
  await expect(section.getByRole('button',{name:'翻译此影片'})).toBeEnabled()
})

test('disabled AI explains how to configure translation', async ({page}) => {
  await seedLibrary(page);await page.goto('/');await page.locator('button.pcard').first().click()
  await page.getByRole('button',{name:'翻译此影片'}).click()
  await expect(page.getByRole('alert')).toContainText('AI 汉化')
})
