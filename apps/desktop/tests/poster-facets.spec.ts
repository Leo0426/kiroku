import { expect, test } from '@playwright/test'
import { seedLibrary } from './helpers'

test.beforeEach(async ({ page }) => seedLibrary(page))

const doneCount = (page: import('@playwright/test').Page) => page.getByRole('tab', { name: /已整理/ })

test('poster wall filters by actor, tag and badge, combined and removable', async ({ page }) => {
  await page.goto('/')
  const facets = await page.evaluate(async () => (await import('/src/lib/api.ts')).api.libraryFacets())
  const [actor, actorCount] = facets.actors[0]
  const [tag] = facets.tags[0]
  const both = await page.evaluate(async ([actor, tag]) => (await (await import('/src/lib/api.ts')).api.libraryList({ filters: [{ kind: 'actor', value: actor }, { kind: 'tag', value: tag }], limit: 1 })).total, [actor, tag])

  await page.getByRole('button', { name: '筛选', exact: true }).click()
  await page.getByLabel('按演员筛选').selectOption(actor)
  await expect(doneCount(page)).toContainText(String(actorCount))
  await expect(page.getByRole('button', { name: `移除筛选 演员 ${actor}` })).toBeVisible()
  await page.getByLabel('按标签筛选').selectOption(tag)
  await expect(doneCount(page)).toContainText(String(both))
  await page.getByRole('button', { name: `移除筛选 演员 ${actor}` }).click()
  await expect(page.getByRole('button', { name: `移除筛选 演员 ${actor}` })).toHaveCount(0)
  await page.getByRole('button', { name: '清空筛选', exact: true }).click()
  await expect(doneCount(page)).toContainText(String(facets.total))

  await page.getByLabel('按角标筛选').selectOption('subtitle')
  await expect(page.getByRole('button', { name: '移除筛选 角标 中字' })).toBeVisible()
})
