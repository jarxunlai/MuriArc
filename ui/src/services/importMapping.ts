export type EditableImportMapping = Record<string, string | null>

export interface CanonicalImportMapping {
  columns: Record<string, string>
}

export function editableMappingFromCanonical(
  headers: string[],
  mapping: CanonicalImportMapping,
): EditableImportMapping {
  const targetBySource = new Map(
    Object.entries(mapping.columns).map(([target, source]) => [source, target]),
  )
  return Object.fromEntries(headers.map((source) => [source, targetBySource.get(source) ?? null]))
}

export function assignImportTarget(
  mapping: EditableImportMapping,
  source: string,
  target: string | null,
): EditableImportMapping {
  const next = { ...mapping }
  if (target) {
    for (const [otherSource, currentTarget] of Object.entries(next)) {
      if (otherSource !== source && currentTarget === target) next[otherSource] = null
    }
  }
  next[source] = target || null
  return next
}

export function canonicalMappingFromEditable(
  mapping: EditableImportMapping,
): CanonicalImportMapping {
  const columns: Record<string, string> = {}
  for (const [source, target] of Object.entries(mapping)) {
    if (target) columns[target] = source
  }
  return { columns }
}

export function sameCanonicalMapping(
  left: CanonicalImportMapping,
  right: CanonicalImportMapping,
): boolean {
  const leftEntries = Object.entries(left.columns).sort(([a], [b]) => a.localeCompare(b))
  const rightEntries = Object.entries(right.columns).sort(([a], [b]) => a.localeCompare(b))
  return JSON.stringify(leftEntries) === JSON.stringify(rightEntries)
}
