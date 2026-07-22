import { describe, expect, it } from 'vitest'
import type { GenotypingRecord } from './models'
import { currentGenotypingRecords } from './genetics'

function record(
  id: string,
  genotypeDefinitionId: string,
  createdAt: string,
  overrides: Partial<GenotypingRecord> = {},
): GenotypingRecord {
  return {
    id,
    animalId: 'animal-1',
    genotypeDefinitionId,
    state: 'unknown',
    revision: 1,
    createdAt,
    updatedAt: createdAt,
    ...overrides,
  }
}

describe('currentGenotypingRecords', () => {
  it('keeps the latest non-void record for each definition', () => {
    const rows = currentGenotypingRecords([
      record('record-a1', 'definition-a', '2026-07-01T00:00:00Z'),
      record('record-a2', 'definition-a', '2026-07-02T00:00:00Z', { state: 'expected' }),
      record('record-a3', 'definition-a', '2026-07-03T00:00:00Z', {
        state: 'confirmed',
        voidedAt: '2026-07-04T00:00:00Z',
        voidReason: 'sample mismatch',
      }),
      record('record-b1', 'definition-b', '2026-07-02T00:00:00Z', { state: 'rejected' }),
    ])

    expect(rows.map((row) => [row.genotypeDefinitionId, row.id, row.state])).toEqual([
      ['definition-a', 'record-a2', 'expected'],
      ['definition-b', 'record-b1', 'rejected'],
    ])
  })
})
