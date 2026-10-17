import type { Page } from '@playwright/test'

/** A fresh browser context uses only synthetic media, never local user settings/files. */
export async function seedLibrary(page: Page) {
  await page.addInitScript(() => {
    localStorage.setItem('kiroku-mock-settings-v2', JSON.stringify({ library_root: '/mock/library' }))
  })
}

/** Inject failures at the backend facade while exercising the real Svelte UI. */
export async function overrideApi(page: Page, script: string) {
  await page.route('**/src/lib/api.ts*', async (route) => {
    const response = await route.fetch()
    await route.fulfill({ response, body: (await response.text()) + '\n' + script })
  })
}
