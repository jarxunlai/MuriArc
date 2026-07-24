import { expect, test, type Page } from '@playwright/test'

const tinyPng = (name: string) => ({
  name,
  mimeType: 'image/png',
  buffer: Buffer.from(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=',
    'base64',
  ),
})

async function selectNaiveOption(page: Page, selector: string, option: string | RegExp) {
  await page.locator(selector).click()
  await page.locator('.n-base-select-option').filter({ hasText: option }).last().click()
}

async function expectDataEntryDialogFits(page: Page) {
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 960 })
    await expect.poll(() => page.locator('.data-entry-dialog').evaluate((card) => {
      const cardRect = card.getBoundingClientRect()
      const content = card.querySelector<HTMLElement>('.n-card__content')
      const footer = card.querySelector<HTMLElement>('.n-card__footer')
      return {
        cardContained: cardRect.left >= -1 && cardRect.right <= window.innerWidth + 1,
        cardHasNoOverflow: card.scrollWidth <= card.clientWidth + 1,
        contentHasNoOverflow: !content || content.scrollWidth <= content.clientWidth + 1,
        footerHasNoOverflow: !footer || footer.scrollWidth <= footer.clientWidth + 1,
        documentHasNoOverflow:
          document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1,
      }
    })).toEqual({
      cardContained: true,
      cardHasNoOverflow: true,
      contentHasNoOverflow: true,
      footerHasNoOverflow: true,
      documentHasNoOverflow: true,
    })
  }
}

test('以笼位视图启动并使用适配设备的导航', async ({ page }, testInfo) => {
  await page.goto('/')

  await expect(page).toHaveTitle(/MuriArc/)
  await expect(page.getByRole('heading', { name: '笼位视图' })).toBeVisible()
  await expect(page.getByText('A01', { exact: true })).toBeVisible()

  const compact = testInfo.project.name !== 'desktop-chromium'
  await expect(page.getByRole('navigation', { name: '移动端主导航', exact: true })).toBeVisible({ visible: compact })
  await expect(page.getByRole('navigation', { name: '主导航', exact: true })).toBeVisible({ visible: !compact })
})

test('可以创建空笼且刷新后的视图立即可见', async ({ page }) => {
  await page.goto('/#/cages')
  await page.getByRole('button', { name: '新增笼位' }).click()

  const dialog = page.getByRole('dialog')
  await dialog.locator('input').nth(0).fill('T99')
  await dialog.locator('input').nth(1).fill('SPF-T')
  await dialog.locator('input').nth(2).fill('R9')
  await dialog.getByRole('button', { name: '创建笼位' }).click()

  await expect(page.getByText('笼位已创建')).toBeVisible()
  const card = page.locator('.cage-card').filter({ hasText: 'T99' })
  await expect(card).toContainText('SPF-T · R9')
  await expect(card).toContainText('空笼')
})

test('可以通过真实 Gateway 入口登记新动物', async ({ page }, testInfo) => {
  await page.goto('/#/animals')
  await page.getByRole('button', { name: '新增小鼠' }).click()

  const dialog = page.getByRole('dialog')
  await dialog.getByPlaceholder('例如 M-26001').fill('M-999')
  await dialog.getByPlaceholder('例如 C57BL/6J').fill('BALB/c')
  await dialog.getByRole('button', { name: '登记小鼠' }).click()

  await expect(page.getByText('已登记小鼠 M-999')).toBeVisible()
  await page.getByPlaceholder('搜索编号、品系、基因型或项目').fill('M-999')
  const compact = testInfo.project.name !== 'desktop-chromium'
  const desktopTable = page.locator('.table-wrap')
  const mobileCards = page.getByRole('region', { name: '小鼠卡片列表' })
  await expect(desktopTable).toBeVisible({ visible: !compact })
  await expect(mobileCards).toBeVisible({ visible: compact })
  const createdAnimal = compact
    ? mobileCards.getByRole('button', { name: '查看动物 M-999', exact: true })
    : desktopTable.getByText('M-999', { exact: true })
  await expect(createdAnimal).toBeVisible()
})

test('动物档案展示身份与可追溯时间线', async ({ page }) => {
  await page.goto('/#/animals?animal=animal-001')

  const drawer = page.locator('.n-drawer')
  await expect(drawer.getByText('M-26001', { exact: true })).toBeVisible()
  await expect(drawer.getByText('时间线', { exact: true })).toBeVisible()
  await expect(drawer.getByText('记录体重')).toBeVisible()
  await expect(drawer.getByText(/数据来源：人工录入/)).toBeVisible()
})

test('动物详情各页签使用真实空状态并可登记最小样本', async ({ page }) => {
  await page.goto('/#/animals?animal=animal-003')

  const drawer = page.locator('.n-drawer')
  await expect(drawer.getByText('M-26003', { exact: true })).toBeVisible()
  const tab = (name: string) => drawer.locator('.n-tabs-tab').filter({ hasText: name })

  await tab('实验').click()
  await expect(drawer.getByText('尚未参与实验', { exact: true })).toBeVisible()

  await tab('测量').click()
  await expect(drawer.getByText('暂无测量记录', { exact: true })).toBeVisible()

  await tab('繁育').click()
  await expect(drawer.getByText('尚未登记父母或后代关系', { exact: true })).toBeVisible()
  await expect(drawer.getByRole('button', { name: '管理谱系' })).toBeVisible()

  await tab('样本').click()
  await drawer.getByRole('button', { name: '登记样本' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByPlaceholder('例如 lung tissue').fill('lung tissue')
  await dialog.getByPlaceholder('例如 -80℃ A / Box 3 / A2').fill('-80℃ A / Box 3 / A2')
  await dialog.getByRole('button', { name: '确认登记' }).click()

  await expect(page.getByText('样本已登记', { exact: true })).toBeVisible()
  await expect(drawer.getByText('lung tissue', { exact: true })).toBeVisible()
  await expect(drawer.getByText('-80℃ A / Box 3 / A2', { exact: true })).toBeVisible()

  await tab('附件').click()
  await expect(drawer.getByText('暂无关联附件', { exact: true })).toBeVisible()

  await tab('审计').click()
  await expect(drawer.getByText('暂无可见审计记录', { exact: true })).toBeVisible()
  await expect(drawer.getByText('暂无可见来源记录', { exact: true })).toBeVisible()
})

test('高风险操作的确认触发按钮在生产入口中可见', async ({ page }) => {
  await page.goto('/#/experiments')

  const experiment = page.locator('.experiment-card').filter({ hasText: 'DEMO-2026-01' })
  await experiment.getByRole('button', { name: '打开实验' }).click()

  await expect(page).toHaveURL(/#\/experiments\/[^/]+\/overview/)
  const workspace = page.locator('.experiment-workspace')
  await expect(workspace.getByRole('button', { name: '完成实验' })).toBeVisible()
  await expect(workspace.getByRole('button', { name: '取消实验' })).toBeVisible()
  await expect(workspace.getByRole('navigation', { name: '实验工作区导航' })).toBeVisible()
  await expect(workspace.getByRole('link', { name: '数据工作表' })).toBeVisible()
})

test('导入文件先进入字段匹配和冲突预览', async ({ page }) => {
  await page.goto('/#/data')
  await page.locator('input[type="file"]').setInputFiles('e2e/fixtures/measurements.csv')

  await expect(page.getByText('measurements.csv', { exact: true })).toBeVisible()
  await expect(page.locator('.steps li.active')).toContainText('检查与预览')
  const blocking = page.locator('.validation-note.blocking')
  await expect(blocking).toContainText(/个阻断错误/)
  await expect(blocking).toContainText('动物编号 M-26001 已存在')

  const confirm = page.getByRole('button', { name: '确认预览并事务写入' })
  await expect(confirm).toBeDisabled()
  await expect(page.getByText('导入已完成', { exact: true })).not.toBeVisible()
})

test('无冲突 CSV 可以从预览完成事务确认', async ({ page }) => {
  await page.goto('/#/data')
  await page.locator('input[type="file"]').setInputFiles('e2e/fixtures/animals-new.csv')

  await expect(page.getByText('animals-new.csv', { exact: true })).toBeVisible()
  await expect(page.locator('.steps li.active')).toContainText('检查与预览')
  await expect(page.locator('.validation-ok')).toContainText('校验通过，可以事务写入')

  const confirm = page.getByRole('button', { name: '确认预览并事务写入' })
  await expect(confirm).toBeEnabled()
  await confirm.click()

  await expect(page.getByText('导入已完成', { exact: true })).toBeVisible()
  await expect(page.locator('.created')).toContainText('动物 2 · 事件 2 · 测量 0')
  await expect(page.locator('.steps li.active')).toContainText('事务写入')
})

test('测量 CSV 绑定明确实验后以草稿测量计数写入', async ({ page }) => {
  await page.goto('/#/data')
  await page.locator('.import-selection').getByText('实验测量', { exact: true }).click()
  await page.locator('.import-selection .n-select').click()
  await page.getByText(/GeneA 抑制对 DEMO 进展的影响/).click()
  await page.locator('input[type="file"]').setInputFiles('e2e/fixtures/measurements-valid.csv')

  await expect(page.getByText('measurements-valid.csv', { exact: true })).toBeVisible()
  await expect(page.locator('.file-summary')).toContainText('实验测量')
  await expect(page.locator('.validation-ok')).toContainText('校验通过，可以事务写入')
  await page.getByRole('button', { name: '确认预览并事务写入' }).click()

  await expect(page.getByText('导入已完成', { exact: true })).toBeVisible()
  await expect(page.locator('.created')).toContainText('动物 0 · 事件 0 · 测量 1')
})

test('动物导入与独立指南在目标视口无步骤重叠和页面横向滚动', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', '由单一浏览器覆盖四个明确验收宽度')

  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 960 })
    await page.goto('/#/animal-data')

    await expect(page.getByRole('heading', { name: '动物数据' })).toBeVisible()
    await expect(page.locator('.import-resources')).toBeVisible()
    await expect(page.locator('.schema-guide')).toHaveCount(0)
    await expect(page.locator('.import-selection')).toHaveCount(0)

    const stepLayoutIsSafe = await page.locator('.steps').evaluate((steps) => {
      const items = Array.from(steps.querySelectorAll('li'))
      const visibleLabels = items
        .map((item) => ({
          item: item.getBoundingClientRect(),
          marker: item.querySelector('i')?.getBoundingClientRect(),
          label: item.querySelector('span')?.getBoundingClientRect(),
        }))
        .filter((entry) => entry.label && entry.label.width > 0 && entry.label.height > 0)
      return visibleLabels.every((entry, index) => {
        if (!entry.marker || !entry.label) return false
        const contained = entry.label.left >= entry.item.left - 1
          && entry.label.right <= entry.item.right + 1
        const belowMarker = entry.label.top >= entry.marker.bottom - 1
        const next = visibleLabels[index + 1]?.label
        const separateFromNext = !next || entry.label.right <= next.left + 1
        return contained && belowMarker && separateFromNext
      })
    })
    expect(stepLayoutIsSafe).toBe(true)

    const importPanel = await page.locator('.import-panel').boundingBox()
    const principles = await page.locator('.principles').boundingBox()
    expect(importPanel).not.toBeNull()
    expect(principles).not.toBeNull()
    if (width <= 1180) {
      expect(principles!.y).toBeGreaterThanOrEqual(importPanel!.y + importPanel!.height - 1)
    } else {
      expect(principles!.x).toBeGreaterThan(importPanel!.x)
      expect(Math.abs(principles!.y - importPanel!.y)).toBeLessThanOrEqual(1)
    }

    expect(await page.evaluate(() =>
      document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1,
    )).toBe(true)

    await page.goto('/#/animal-data/import-guide')
    await expect(page.getByRole('heading', { name: '动物导入指南' })).toBeVisible()
    await expect(page.locator('.example-table-scroll tbody tr')).toHaveCount(4)
    await expect(page.getByTestId('download-xlsx-example')).toBeDisabled()
    await expect(page.getByText(/当前运行环境仅提供 CSV 模板/)).toBeVisible()
    expect(await page.evaluate(() =>
      document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1,
    )).toBe(true)

    if (width === 1440) {
      await expect(page.locator('.nav-item.active')).toContainText('动物数据')
    }
  }
})

test('动物 Registry 导出和完整业务归档快照都会触发浏览器下载', async ({ page }) => {
  await page.goto('/#/data')

  await page.getByRole('button', { name: '配置动物导出' }).click()
  const exportDialog = page.getByRole('dialog')
  await expect(exportDialog).toContainText('配置动物业务导出')
  const [exportDownload] = await Promise.all([
    page.waitForEvent('download'),
    exportDialog.getByRole('button', { name: '生成并下载' }).click(),
  ])
  expect(exportDownload.suggestedFilename()).toBe('animals-demo.csv')
  await expect(page.getByText(/动物业务导出已生成/)).toBeVisible()

  const snapshotButton = page.getByRole('button', { name: '创建完整归档快照' })
  await expect(snapshotButton).toBeEnabled()
  const snapshotDownload = page.waitForEvent('download')
  await snapshotButton.click()
  expect((await snapshotDownload).suggestedFilename()).toBe('muriarc-demo-snapshot.json')
  await expect(page.getByText(/完整业务归档快照已生成/)).toBeVisible()
  await expect(page.getByText(/当前不可 restore\/apply/)).toBeVisible()
})

test('AI 工作页保留上下文、回答和数据引用', async ({ page }) => {
  await page.goto('/#/ai')
  await expect(page.getByTestId('conversation-mode-status')).toContainText('请求 Full（待启用）')
  await expect(page.getByTestId('conversation-mode-status')).toContainText('实际 尚未开始')
  const prompt = page.getByLabel('发送给 AI 的消息')
  await prompt.fill('总结进行中的实验')
  await prompt.press('Enter')

  const fullDialog = page.getByRole('dialog')
  await expect(fullDialog).toContainText('以 Full 请求开始新会话')
  await fullDialog.getByRole('checkbox').check()
  await fullDialog.getByRole('button', { name: '确认启用' }).click()
  await expect(page.locator('.message.user .bubble p')).toHaveText('总结进行中的实验')
  await expect(page.getByText(/浏览器演示不会读取正式数据库/)).toBeVisible()
  await expect(page.getByRole('link', { name: '动物 M-26006' })).toBeVisible()
  await expect(page.getByText(/已调用 1 个安全领域工具/)).toBeVisible()
  await expect(page.getByTestId('conversation-mode-status')).toContainText('实际 Full')

  await page.getByRole('button', { name: '新会话', exact: true }).click()
  await expect(page.locator('.message.user')).toHaveCount(0)
  await expect(page.getByText(/选择科研项目后/)).toBeVisible()
})

test('AI 浮动工作台可由键盘移动缩放并在目标视口内保持可用', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', '由单一浏览器覆盖四个明确验收宽度')

  await page.addInitScript(() => localStorage.clear())
  await page.setViewportSize({ width: 1440, height: 960 })
  await page.goto('/#/cages')
  await page.getByRole('button', { name: '问 AI' }).click()

  const workbench = page.getByRole('dialog', { name: 'MuriArc AI 工作台' })
  const dragHandle = page.getByLabel(/AI 工作台拖动区域/)
  await expect(workbench).toBeVisible()
  const initial = await workbench.boundingBox()
  expect(initial).not.toBeNull()

  await dragHandle.focus()
  await dragHandle.press('Alt+ArrowLeft')
  await expect.poll(async () => (await workbench.boundingBox())?.x ?? initial!.x)
    .toBeLessThan(initial!.x)
  const moved = await workbench.boundingBox()
  expect(moved).not.toBeNull()

  await dragHandle.press('Control+Alt+ArrowLeft')
  await expect.poll(async () => (await workbench.boundingBox())?.width ?? moved!.width)
    .toBeLessThan(moved!.width)
  const resized = await workbench.boundingBox()
  expect(resized).not.toBeNull()

  await page.getByRole('button', { name: '最大化' }).click()
  await expect(page.getByRole('button', { name: '还原窗口' })).toBeVisible()
  await page.getByRole('button', { name: '复位大小和位置' }).click()

  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 960 })
    await expect(workbench).toBeVisible()
    const bounds = await workbench.boundingBox()
    expect(bounds).not.toBeNull()
    expect(bounds!.x).toBeGreaterThanOrEqual(-1)
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width + 1)
    expect(await workbench.evaluate((element) => element.scrollWidth <= element.clientWidth + 1))
      .toBe(true)
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1))
      .toBe(true)
  }
})

test('聊天图片明确覆盖无默认、视觉中转与当前模型直接视觉', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', '单一浏览器覆盖模型配置与两条视觉路径')

  await page.goto('/#/settings')
  await page.getByRole('navigation', { name: '设置导航' })
    .getByRole('button', { name: 'AI 与模型' })
    .click()
  await page.locator('.model-row').filter({ hasText: '本地视觉模型' }).click()
  await page.getByRole('button', { name: '默认视觉模型' }).click()
  await expect(page.getByText('默认视觉模型已更新', { exact: true })).toBeVisible()

  await page.goto('/#/ai')
  await expect(page.getByTestId('conversation-model-select')).toBeVisible()
  await selectNaiveOption(page, '[data-testid="conversation-mode-select"]', 'Ask')
  await page.getByLabel('选择最多八张聊天图片').setInputFiles(tinyPng('relay.png'))

  await expect(page.getByRole('status')).toContainText('请明确选择一个可用的视觉模型')
  await expect(page.getByTestId('ai-composer-send')).toBeDisabled()
  await selectNaiveOption(
    page,
    '[aria-label="聊天图片视觉中转模型"]',
    /本地视觉模型/,
  )
  await page.getByTestId('ai-composer-input').fill('请通过视觉中转读取这张图片')
  await page.getByTestId('ai-composer-send').click()

  await expect(page.locator('.message.assistant .bubble').last())
    .toContainText('视觉中转路径读取 1 张演示图片')
  const relayTrace = page.locator('.message.assistant .tool-trace').last()
  await relayTrace.locator('summary').click()
  await expect(relayTrace).toContainText('视觉观察')
  await expect(relayTrace).toContainText('最终回答')

  await page.getByRole('button', { name: '新会话', exact: true }).click()
  await selectNaiveOption(
    page,
    '[data-testid="conversation-model-select"]',
    /本地视觉模型/,
  )
  await selectNaiveOption(page, '[data-testid="conversation-mode-select"]', 'Ask')
  await page.getByLabel('选择最多八张聊天图片').setInputFiles(tinyPng('direct.png'))
  await expect(page.getByText('当前对话模型支持视觉，将直接读取图片并回答。')).toBeVisible()
  await page.getByTestId('ai-composer-input').fill('请由当前模型直接读取这张图片')
  await page.getByTestId('ai-composer-send').click()

  await expect(page.locator('.message.assistant .bubble').last())
    .toContainText('当前模型直接视觉路径读取 1 张演示图片')
  const directTrace = page.locator('.message.assistant .tool-trace').last()
  await directTrace.locator('summary').click()
  await expect(directTrace).toContainText('直接视觉')
})

test('实验数据图片候选支持多图、恢复、拒绝与人工批准', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', '单一浏览器覆盖四个宽度与完整候选生命周期')
  test.setTimeout(60_000)

  await page.goto('/#/experiments')
  const experiment = page.locator('.experiment-card').filter({ hasText: 'DEMO-2026-01' })
  await experiment.getByRole('button', { name: '打开实验' }).click()

  await page.getByRole('link', { name: '追溯' }).click()
  await page.getByRole('button', { name: '记录实验事件' }).click()
  let dialog = page.getByRole('dialog')
  await dialog.getByPlaceholder('例如：设备异常、提前退出或阶段完成').fill('视觉采集节点')
  await dialog.getByRole('button', { name: '保存事件' }).click()
  await expect(page.getByText('实验事件已创建', { exact: true })).toBeVisible()

  await page.getByRole('link', { name: '实验设计' }).click()
  await page.getByRole('button', { name: '添加数据列' }).click()
  dialog = page.getByRole('dialog')
  await dialog.getByPlaceholder('body_weight').fill('vision_note')
  await dialog.getByPlaceholder('体重').fill('视觉观察')
  await dialog.getByRole('button', { name: '添加数据列' }).click()
  await expect(page.getByText('观察定义已创建', { exact: true })).toBeVisible()

  await page.getByRole('link', { name: '数据工作表' }).click()
  await page.getByRole('button', { name: '录入实验级数据' }).click()
  dialog = page.getByRole('dialog', { name: '录入实验数据' })
  await page.getByText('图片识别候选', { exact: true }).click()

  const fileInput = page.getByLabel('选择当前数据单元的图片')
  await fileInput.setInputFiles([tinyPng('cell-a.png'), tinyPng('cell-b.png')])
  await expect(page.getByAltText(/当前数据单元图片：/)).toHaveCount(2)
  await page.getByRole('button', { name: '移除图片 cell-b.png' }).click()
  await expect(page.getByAltText(/当前数据单元图片：/)).toHaveCount(1)
  await fileInput.setInputFiles(tinyPng('cell-b.png'))
  await expect(page.getByAltText(/当前数据单元图片：/)).toHaveCount(2)
  await expectDataEntryDialogFits(page)

  await page.getByRole('button', { name: '生成候选' }).click()
  await expect(page.locator('.candidate-review')).toContainText('AI 候选（可编辑）')
  await expect(page.locator('.candidate-review')).toContainText('2 张证据')
  await expectDataEntryDialogFits(page)

  await page.getByRole('button', { name: '取消', exact: true }).click()
  await expect(dialog).not.toBeVisible()
  await page.getByRole('button', { name: '录入实验级数据' }).click()
  dialog = page.getByRole('dialog', { name: '录入实验数据' })
  await expect(page.locator('.candidate-review')).toContainText('2 张证据')
  await page.getByRole('button', { name: '放弃候选并释放图片' }).click()
  await expect(page.getByText('已放弃候选并释放全部私人暂存图片', { exact: true }))
    .toBeVisible()
  await expect(page.locator('.candidate-review')).toHaveCount(0)

  const secondFileInput = page.getByLabel('选择当前数据单元的图片')
  await secondFileInput.setInputFiles([tinyPng('approved-a.png'), tinyPng('approved-b.png')])
  await page.getByRole('button', { name: '生成候选' }).click()
  await expect(page.locator('.candidate-review')).toContainText('2 张证据')
  await page.getByRole('checkbox', {
    name: '我已核对当前数据单元、候选值和全部图片证据，并批准写入正式数据',
  }).check()
  await page.getByRole('button', { name: '批准并正式写入' }).click()

  await expect(dialog).not.toBeVisible()
  await expect(page.getByText(
    '已由人工批准并原子写入 Observation、附件、Audit 与 Provenance',
    { exact: true },
  )).toBeVisible()
  await page.getByText('实验级记录', { exact: true }).click()
  await expect(page.locator('.experiment-records')).toContainText('演示识别候选')
})

test('AI 模型与模式控制在 375 768 1024 1440 宽度无横向溢出', async ({ page }) => {
  await page.goto('/#/ai')
  await expect(page.getByTestId('conversation-model-select')).toBeVisible()

  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 960 })
    await expect(page.getByTestId('conversation-mode-select')).toBeVisible()
    await expect(page.getByTestId('conversation-mode-status')).toBeVisible()

    const layout = await page.locator('.ai-conversation').evaluate((root) => {
      const rootRect = root.getBoundingClientRect()
      const selectors = [
        '.context-strip',
        '.conversation-controls',
        '.model-field',
        '.mode-field',
        '.mode-status',
        '.input-wrap',
      ]
      const contained = selectors.every((selector) => {
        const element = root.querySelector(selector)
        if (!element) return false
        const rect = element.getBoundingClientRect()
        return rect.left >= rootRect.left - 1 && rect.right <= rootRect.right + 1
      })
      return {
        contained,
        rootHasNoOverflow: root.scrollWidth <= root.clientWidth + 1,
        documentHasNoOverflow:
          document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1,
      }
    })

    expect(layout).toEqual({
      contained: true,
      rootHasNoOverflow: true,
      documentHasNoOverflow: true,
    })
  }
})

test('手机端使用选择动物再选择目标笼位的转笼流程', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'mobile-chromium', '仅验证手机交互出口')

  await page.goto('/#/cages')
  const sourceCard = page.locator('.cage-card').filter({ hasText: 'A01' })
  await sourceCard.getByRole('checkbox').first().click()
  await page.getByRole('button', { name: '移动到笼位' }).click()

  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('目标笼位').click()
  await page.getByText('A03 · 0/5', { exact: true }).click()
  const confirmMove = dialog.getByRole('button', { name: '确认移动', exact: true })
  await expect(confirmMove).toBeEnabled()
  await confirmMove.click()

  await expect(page.getByText('已移动至笼位 A03')).toBeVisible()
  await expect(page.locator('.cage-card').filter({ hasText: 'A03' })).toContainText('M-26001')
})
