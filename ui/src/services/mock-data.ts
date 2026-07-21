import type { Animal, Cage, DataJob, Experiment } from '@/domain/models'

export const seedCages: Cage[] = [
  { id: 'cage-a01', code: 'A01', room: 'SPF-A', rack: 'R1', capacity: 6, animalIds: ['animal-001', 'animal-002', 'animal-003', 'animal-004'], status: 'normal', summary: 'C57BL/6J · 雄 · WT' },
  { id: 'cage-a02', code: 'A02', room: 'SPF-A', rack: 'R1', capacity: 6, animalIds: ['animal-005', 'animal-006', 'animal-007'], status: 'attention', summary: 'GeneA cKO · 雌', note: '1 只待确认基因型' },
  { id: 'cage-a03', code: 'A03', room: 'SPF-A', rack: 'R1', capacity: 5, animalIds: [], status: 'empty', summary: '空笼' },
  { id: 'cage-b01', code: 'B01', room: 'SPF-A', rack: 'R2', capacity: 5, animalIds: ['animal-008', 'animal-009', 'animal-010'], status: 'normal', summary: 'BALB/c · 雌 · 实验组' },
  { id: 'cage-b02', code: 'B02', room: 'SPF-A', rack: 'R2', capacity: 5, animalIds: ['animal-011', 'animal-012'], status: 'normal', summary: 'C57BL/6J · 繁育' },
  { id: 'cage-c01', code: 'C01', room: '屏障区-B', rack: 'R1', capacity: 5, animalIds: ['animal-013', 'animal-014', 'animal-015', 'animal-016', 'animal-017'], status: 'attention', summary: 'DEMO · Day 7', note: '今日需记录体重' },
]

const makeTimeline = (code: string, birthDate: string, cage: string) => [
  { id: `${code}-e3`, at: '2026-07-16T09:20:00+08:00', type: 'measurement' as const, title: '记录体重', detail: '体重 23.4 g，数据来源：人工录入', operator: '本地操作者' },
  { id: `${code}-e2`, at: '2026-07-10T14:05:00+08:00', type: 'transfer' as const, title: '转入笼位', detail: `转入 ${cage}`, operator: '本地操作者' },
  { id: `${code}-e1`, at: `${birthDate}T08:30:00+08:00`, type: 'birth' as const, title: '出生登记', detail: '由繁育记录创建动物档案', operator: '本地操作者' },
]

const animalRows: Array<[string, string, Animal['sex'], string, string, string, Animal['status'], string | null, string[]]> = [
  ['animal-001', 'M-26001', 'male', 'C57BL/6J', 'WT', '2026-03-18', 'active', 'cage-a01', ['种群维护']],
  ['animal-002', 'M-26002', 'male', 'C57BL/6J', 'WT', '2026-03-18', 'active', 'cage-a01', ['种群维护']],
  ['animal-003', 'M-26003', 'male', 'C57BL/6J', 'WT', '2026-03-18', 'experiment', 'cage-a01', ['DEMO-GeneA']],
  ['animal-004', 'M-26004', 'male', 'C57BL/6J', 'WT', '2026-03-18', 'experiment', 'cage-a01', ['DEMO-GeneA']],
  ['animal-005', 'M-26005', 'female', 'GeneAfl/fl', 'GeneAfl/fl', '2026-04-02', 'breeding', 'cage-a02', ['GeneA 繁育']],
  ['animal-006', 'M-26006', 'female', 'GeneAfl/fl', '待确认', '2026-04-02', 'active', 'cage-a02', ['GeneA 繁育']],
  ['animal-007', 'M-26007', 'female', 'GeneAfl/fl', 'GeneAfl/fl', '2026-04-02', 'active', 'cage-a02', ['GeneA 繁育']],
  ['animal-008', 'M-26008', 'female', 'BALB/c', 'WT', '2026-02-25', 'experiment', 'cage-b01', ['DEMO-GeneA']],
  ['animal-009', 'M-26009', 'female', 'BALB/c', 'WT', '2026-02-25', 'experiment', 'cage-b01', ['DEMO-GeneA']],
  ['animal-010', 'M-26010', 'female', 'BALB/c', 'WT', '2026-02-25', 'experiment', 'cage-b01', ['DEMO-GeneA']],
  ['animal-011', 'M-26011', 'male', 'C57BL/6J', 'Cre+', '2025-12-12', 'breeding', 'cage-b02', ['GeneA 繁育']],
  ['animal-012', 'M-26012', 'female', 'C57BL/6J', 'fl/fl', '2026-01-04', 'breeding', 'cage-b02', ['GeneA 繁育']],
  ['animal-013', 'M-26013', 'female', 'BALB/c', 'WT', '2026-02-20', 'experiment', 'cage-c01', ['DEMO-GeneA']],
  ['animal-014', 'M-26014', 'female', 'BALB/c', 'WT', '2026-02-20', 'experiment', 'cage-c01', ['DEMO-GeneA']],
  ['animal-015', 'M-26015', 'female', 'BALB/c', 'WT', '2026-02-20', 'experiment', 'cage-c01', ['DEMO-GeneA']],
  ['animal-016', 'M-26016', 'female', 'BALB/c', 'WT', '2026-02-20', 'experiment', 'cage-c01', ['DEMO-GeneA']],
  ['animal-017', 'M-26017', 'female', 'BALB/c', 'WT', '2026-02-20', 'experiment', 'cage-c01', ['DEMO-GeneA']],
]

export const seedAnimals: Animal[] = animalRows.map(([id, code, sex, strain, genotype, birthDate, status, cageId, projectNames], index) => ({
  id,
  code,
  legacyCode: index < 3 ? String(101 + index) : undefined,
  sex,
  strain,
  genotype,
  birthDate,
  status,
  cageId,
  projectNames,
  weight: 20.8 + index * 0.32,
  timeline: makeTimeline(code, birthDate, seedCages.find((cage) => cage.id === cageId)?.code ?? '未分配'),
}))

export const seedExperiments: Experiment[] = [
  {
    id: 'exp-001', projectId: 'demo-project-1', code: 'DEMO-2026-01', name: 'GeneA 抑制对 DEMO 进展的影响', project: 'DEMO-GeneA',
    status: 'active', startDate: '2026-07-09', animalCount: 12, completedSteps: 2, totalSteps: 4,
    groups: [{ name: 'Vehicle', count: 4, color: '#7398bd' }, { name: 'Compound-A', count: 4, color: '#009ca6' }, { name: 'Compound-B', count: 4, color: '#ef9f27' }],
    nextAction: '今日记录 Day 7 体重与胸水量', revision: 1,
  },
  {
    id: 'exp-002', projectId: 'demo-project-2', code: 'BREED-2026-03', name: 'GeneA 条件敲除繁育验证', project: 'GeneA 繁育',
    status: 'active', startDate: '2026-06-12', animalCount: 8, completedSteps: 3, totalSteps: 5,
    groups: [{ name: 'Cre+', count: 3, color: '#5b7db1' }, { name: 'fl/fl', count: 5, color: '#8d9c65' }],
    nextAction: '2 份基因型结果待复核', revision: 1,
  },
  {
    id: 'exp-003', projectId: 'demo-project-1', code: 'PILOT-2026-02', name: '给药剂量预实验', project: 'DEMO-GeneA',
    status: 'completed', startDate: '2026-05-02', animalCount: 6, completedSteps: 4, totalSteps: 4,
    groups: [{ name: 'Low', count: 3, color: '#7398bd' }, { name: 'High', count: 3, color: '#009ca6' }], revision: 1,
  },
]

export const seedDataJobs: DataJob[] = [
  { id: 'job-001', name: 'DEMO_Day7_体重.xlsx', kind: 'import', status: 'needs-review', progress: 100, createdAt: '今天 09:42', detail: '17 行已匹配，2 行需要确认编号' },
  { id: 'job-002', name: '动物与实验快照', kind: 'snapshot', status: 'completed', progress: 100, createdAt: '昨天 18:10', detail: '包含 17 只示例动物与附件清单' },
  { id: 'job-003', name: 'DEMO-2026-01_数据导出.csv', kind: 'export', status: 'completed', progress: 100, createdAt: '07-15 16:24', detail: '42 条测量记录' },
]
