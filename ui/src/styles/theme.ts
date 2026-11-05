import type { GlobalThemeOverrides } from 'naive-ui'
import { branding } from '@/branding'

/**
 * Naive UI derives hover/pressed shades from literal colours, so it cannot read
 * CSS variables. These values mirror tokens.css; theme.test.ts fails if they drift.
 */
export const palette = {
  primary: branding.primaryColor,
  primaryHover: '#1a69b3',
  primaryPressed: '#0b4c8a',
  primarySoft: '#edf3fa',
  bg: '#f7f8fa',
  surface: '#ffffff',
  surfaceMuted: '#fafbfc',
  surfaceHover: '#f3f4f6',
  surfaceSunken: '#f0f1f4',
  border: '#e2e4e9',
  borderStrong: '#c9ccd3',
  text: '#1c2024',
  textSecondary: '#555b65',
  textTertiary: '#697079',
  success: '#22704e',
  warning: '#9c5a00',
  danger: '#b4383e',
} as const

const fontFamily = 'Inter, ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif'
const fontFamilyMono = 'ui-monospace, "Cascadia Mono", "SF Mono", Menlo, Consolas, "Liberation Mono", monospace'

export const themeOverrides: GlobalThemeOverrides = {
  common: {
    fontFamily,
    fontFamilyMono,
    fontSize: '14px',
    borderRadius: '6px',
    borderRadiusSmall: '4px',
    primaryColor: palette.primary,
    primaryColorHover: palette.primaryHover,
    primaryColorPressed: palette.primaryPressed,
    primaryColorSuppl: palette.primaryHover,
    infoColor: palette.primary,
    infoColorHover: palette.primaryHover,
    infoColorPressed: palette.primaryPressed,
    infoColorSuppl: palette.primaryHover,
    successColor: palette.success,
    successColorHover: '#2a8059',
    successColorPressed: '#1a5a3e',
    successColorSuppl: '#2a8059',
    warningColor: palette.warning,
    warningColorHover: '#ad6600',
    warningColorPressed: '#7d4900',
    warningColorSuppl: '#ad6600',
    errorColor: palette.danger,
    errorColorHover: '#c24a50',
    errorColorPressed: '#962c31',
    errorColorSuppl: '#c24a50',
    textColorBase: palette.text,
    textColor1: palette.text,
    textColor2: palette.textSecondary,
    textColor3: palette.textTertiary,
    placeholderColor: palette.textTertiary,
    borderColor: palette.border,
    dividerColor: palette.border,
    bodyColor: palette.bg,
    cardColor: palette.surface,
    modalColor: palette.surface,
    tableHeaderColor: palette.surfaceMuted,
    hoverColor: palette.surfaceHover,
  },
  Button: { heightMedium: '34px', borderRadiusMedium: '6px', borderRadiusSmall: '6px', fontWeight: '500' },
  Input: { heightMedium: '34px', borderRadius: '6px', border: `1px solid ${palette.borderStrong}` },
  Card: { borderRadius: '8px' },
  Tag: { borderRadius: '4px' },
  // Section tabs: quiet neutral underline; the brand blue stays on primary actions.
  Tabs: {
    barColor: palette.text,
    tabTextColorLine: palette.textSecondary,
    tabTextColorHoverLine: palette.text,
    tabTextColorActiveLine: palette.text,
    tabFontWeightActive: '500',
    tabGapMediumLine: '24px',
    tabBorderColor: palette.border,
    tabTextColorSegment: palette.textSecondary,
    tabTextColorActiveSegment: palette.text,
    tabTextColorHoverSegment: palette.text,
    colorSegment: palette.surfaceSunken,
  },
  DataTable: {
    thColor: palette.surfaceMuted,
    thTextColor: palette.textSecondary,
    thFontWeight: '500',
    tdColorHover: palette.surfaceHover,
    borderColor: palette.border,
  },
}
