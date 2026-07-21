import { describe, expect, it } from 'vitest'
import {
  assignImportTarget,
  canonicalMappingFromEditable,
  editableMappingFromCanonical,
  sameCanonicalMapping,
} from './importMapping'

describe('editable import mapping', () => {
  it('moves a canonical target instead of assigning it to two source columns', () => {
    const initial = { code: 'display_id', legacy: null, gender: 'sex' }
    const moved = assignImportTarget(initial, 'legacy', 'display_id')

    expect(moved).toEqual({ code: null, legacy: 'display_id', gender: 'sex' })
    expect(initial).toEqual({ code: 'display_id', legacy: null, gender: 'sex' })
    expect(canonicalMappingFromEditable(moved)).toEqual({
      columns: { display_id: 'legacy', sex: 'gender' },
    })
  })

  it('supports explicitly clearing a source mapping', () => {
    expect(assignImportTarget({ code: 'display_id' }, 'code', null)).toEqual({ code: null })
  })

  it('round-trips canonical mappings and compares key order independently', () => {
    const canonical = { columns: { sex: 'gender', display_id: 'code' } }
    const editable = editableMappingFromCanonical(['code', 'gender', 'note'], canonical)
    expect(editable).toEqual({ code: 'display_id', gender: 'sex', note: null })
    expect(sameCanonicalMapping(canonicalMappingFromEditable(editable), canonical)).toBe(true)
  })
})
