import type { GenotypingRecord } from './models'

export function currentGenotypingRecords(records: GenotypingRecord[]): GenotypingRecord[] {
  const latest = new Map<string, GenotypingRecord>()
  for (const record of records) {
    if (record.voidedAt) continue
    const previous = latest.get(record.genotypeDefinitionId)
    if (!previous
      || record.createdAt > previous.createdAt
      || (record.createdAt === previous.createdAt && record.id > previous.id)) {
      latest.set(record.genotypeDefinitionId, record)
    }
  }
  return [...latest.values()].sort((left, right) =>
    left.genotypeDefinitionId.localeCompare(right.genotypeDefinitionId))
}
