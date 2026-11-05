import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { palette } from './theme'

// Vitest stubs CSS imports (even ?raw), so read stylesheets from disk.
const tokensCss = readFileSync(resolve(__dirname, 'tokens.css'), 'utf8')
const globalCss = readFileSync(resolve(__dirname, 'global.css'), 'utf8')
const components = import.meta.glob('../**/*.vue', { query: '?raw', import: 'default', eager: true }) as Record<string, string>
const sources = [globalCss, ...Object.values(components)]
const tokens = Object.fromEntries(
  [...tokensCss.matchAll(/(--muri-[a-z0-9-]+):\s*([^;]+);/g)].map((match) => [match[1]!, match[2]!.trim()]),
)

function luminance(hex: string): number {
  const channels = [1, 3, 5].map((index) => parseInt(hex.slice(index, index + 2), 16) / 255)
    .map((value) => (value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4))
  return 0.2126 * channels[0]! + 0.7152 * channels[1]! + 0.0722 * channels[2]!
}

function contrast(foreground: string, background: string): number {
  const [light, dark] = [luminance(foreground), luminance(background)].sort((a, b) => b - a)
  return (light! + 0.05) / (dark! + 0.05)
}

describe('design tokens', () => {
  it('keeps the Naive UI palette identical to tokens.css', () => {
    const mirrored: Record<keyof typeof palette, string> = {
      primary: '--muri-primary', primaryHover: '--muri-primary-hover', primaryPressed: '--muri-primary-pressed',
      primarySoft: '--muri-primary-soft', bg: '--muri-bg', surface: '--muri-surface',
      surfaceMuted: '--muri-surface-muted', surfaceHover: '--muri-surface-hover', surfaceSunken: '--muri-surface-sunken',
      border: '--muri-border', borderStrong: '--muri-border-strong', text: '--muri-text',
      textSecondary: '--muri-text-secondary', textTertiary: '--muri-text-tertiary', success: '--muri-success',
      warning: '--muri-warning', danger: '--muri-danger',
    }
    for (const [key, token] of Object.entries(mirrored)) {
      expect(tokens[token], token).toBe(palette[key as keyof typeof palette])
    }
  })

  it('keeps text tokens at WCAG AA contrast on every page surface', () => {
    const surfaces = ['--muri-surface', '--muri-surface-muted', '--muri-bg']
    const texts = ['--muri-text', '--muri-text-secondary', '--muri-text-tertiary', '--muri-primary',
      '--muri-success', '--muri-warning', '--muri-danger', '--muri-warning-text', '--muri-danger-text']
    for (const text of texts) {
      for (const surface of surfaces) {
        expect(contrast(tokens[text]!, tokens[surface]!), `${text} on ${surface}`).toBeGreaterThanOrEqual(4.5)
      }
    }
    for (const [text, surface] of [['--muri-warning-text', '--muri-warning-soft'], ['--muri-danger-text', '--muri-danger-soft'],
      ['--muri-success', '--muri-success-soft'], ['--muri-primary', '--muri-primary-soft'], ['--muri-on-primary', '--muri-primary']]) {
      expect(contrast(tokens[text!]!, tokens[surface!]!), `${text} on ${surface}`).toBeGreaterThanOrEqual(4.5)
    }
  })

  it('defines every token referenced by components', () => {
    const used = new Set(sources.flatMap((source) => [...source.matchAll(/var\((--muri-[a-z0-9-]*[a-z0-9])\b(?!-)/g)].map((match) => match[1]!)))
    expect([...used].filter((token) => !(token in tokens))).toEqual([])
  })
})
