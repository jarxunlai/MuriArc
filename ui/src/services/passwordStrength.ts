export type PasswordStrengthLevel = 'too-short' | 'weak' | 'medium' | 'strong'

export interface PasswordStrength {
  level: PasswordStrengthLevel
  label: '过短' | '弱' | '中' | '强'
  percentage: number
  status: 'error' | 'warning' | 'success'
}

export function passwordCharacterCount(value: string): number {
  return Array.from(value).length
}

export function passwordByteCount(value: string): number {
  return new TextEncoder().encode(value).byteLength
}

/** Advisory only: server acceptance depends on length and control characters, not this level. */
export function passwordStrength(value: string, minChars = 8): PasswordStrength {
  const length = passwordCharacterCount(value)
  if (length < minChars) return { level: 'too-short', label: '过短', percentage: 18, status: 'error' }
  if (length < Math.max(12, minChars + 4)) return { level: 'weak', label: '弱', percentage: 42, status: 'warning' }
  if (length < Math.max(16, minChars + 8)) return { level: 'medium', label: '中', percentage: 68, status: 'warning' }
  return { level: 'strong', label: '强', percentage: 100, status: 'success' }
}

export function passwordPolicyError(value: string, minChars = 8): string | undefined {
  if (passwordCharacterCount(value) < minChars) return `密码至少需要 ${minChars} 个字符`
  if (passwordByteCount(value) > 1024) return '密码不能超过 1024 字节'
  if (Array.from(value).some((character) => /\p{Cc}/u.test(character))) {
    return '密码不能包含控制字符'
  }
  return undefined
}
