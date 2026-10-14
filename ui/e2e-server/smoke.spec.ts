import { expect, test, type Page } from '@playwright/test'

// Real Server smoke: Vue remote gateway -> Axum (session + CSRF) -> Application -> PostgreSQL.
// Credentials come from the environment of the disposable stack; never hard-code them.
const email = process.env.MURIARC_E2E_EMAIL ?? ''
const password = process.env.MURIARC_E2E_PASSWORD ?? ''

// Unique per run so a reused local stack never collides with earlier records.
const runId = Date.now().toString(36).toUpperCase()
const cageCode = `E2E-${runId}`
const animalCode = `M-E2E-${runId}`

test.beforeAll(() => {
  if (!email || !password) {
    throw new Error('MURIARC_E2E_EMAIL and MURIARC_E2E_PASSWORD must point to a disposable Server account')
  }
})

async function login(page: Page) {
  await page.goto('/#/login')
  await page.getByPlaceholder('name@example.org').fill(email)
  await page.getByPlaceholder('输入密码').fill(password)
  await page.getByRole('button', { name: '登录' }).click()
  await expect(page.getByRole('heading', { name: '笼位视图' })).toBeVisible()
}

test('登录后创建笼位、登记动物，并在刷新后读取服务端详情与审计', async ({ page }) => {
  await login(page)

  await page.getByRole('button', { name: '新增笼位' }).click()
  const cageDialog = page.getByRole('dialog')
  await cageDialog.locator('input').nth(0).fill(cageCode)
  await cageDialog.locator('input').nth(1).fill('SPF-E2E')
  await cageDialog.locator('input').nth(2).fill('R1')
  await cageDialog.getByRole('button', { name: '创建笼位' }).click()
  await expect(page.getByText('笼位已创建')).toBeVisible()
  await expect(page.locator('.cage-card').filter({ hasText: cageCode })).toContainText('空笼')

  await page.goto('/#/animals')
  await page.getByRole('button', { name: '新增小鼠' }).click()
  const animalDialog = page.getByRole('dialog')
  await animalDialog.getByPlaceholder('例如 M-26001').fill(animalCode)
  await animalDialog.getByPlaceholder('例如 C57BL/6J').fill('C57BL/6J')
  await animalDialog.getByRole('button', { name: '登记小鼠' }).click()
  await expect(page.getByText(`已登记小鼠 ${animalCode}`)).toBeVisible()

  // A reload drops all client state, so everything below is read back from PostgreSQL.
  await page.reload()
  await page.getByPlaceholder('搜索编号、品系、基因型或项目').fill(animalCode)
  await page.locator('.table-wrap').getByText(animalCode, { exact: true }).click()

  const drawer = page.locator('.n-drawer')
  await expect(drawer.getByText(animalCode, { exact: true })).toBeVisible()
  await expect(drawer.getByText('时间线', { exact: true })).toBeVisible()
  await drawer.locator('.n-tabs-tab').filter({ hasText: '审计' }).click()
  // Registration is a formal write, so the server must have recorded an audit entry.
  const audits = drawer.locator('.audit-list article')
  await expect(audits.first()).toBeVisible()
  await expect(audits.first()).toContainText(/revision \d+/)
})
