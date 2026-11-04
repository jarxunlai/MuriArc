import { describe, expect, it } from 'vitest'

// Naive UI is not registered globally (see main.ts). A template tag without a
// matching import silently renders as an unknown element, so guard every SFC.
const sources = import.meta.glob('./**/*.vue', { query: '?raw', import: 'default', eager: true }) as Record<string, string>

function pascal(tag: string): string {
  return tag.split('-').map((part) => part.charAt(0).toUpperCase() + part.slice(1)).join('')
}

describe('Naive UI local imports', () => {
  it('imports every Naive component used in a template', () => {
    const missing: string[] = []
    for (const [file, source] of Object.entries(sources)) {
      const template = source.slice(source.indexOf('<template>'))
      const used = new Set([...template.matchAll(/<(n-[a-z-]+)/g)].map((match) => pascal(match[1]!)))
      const importBlock = source.match(/import\s*\{([^}]*)\}\s*from 'naive-ui'/)?.[1] ?? ''
      const imported = new Set(importBlock.split(',').map((name) => name.trim()))
      for (const name of used) {
        if (!imported.has(name)) missing.push(`${file}: ${name}`)
      }
    }
    expect(missing).toEqual([])
  })
})
