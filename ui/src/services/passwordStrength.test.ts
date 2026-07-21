import { describe, expect, it } from 'vitest'
import { passwordByteCount, passwordPolicyError, passwordStrength } from './passwordStrength'

describe('password advice and acceptance boundary', () => {
  it('keeps strength advisory while accepting any eight non-control characters', () => {
    expect(passwordStrength('1234567').level).toBe('too-short')
    expect(passwordStrength('12345678').level).toBe('weak')
    expect(passwordStrength('abcdefghijkl').level).toBe('medium')
    expect(passwordStrength('abcdefghijklmnop').level).toBe('strong')
    expect(passwordPolicyError('12345678')).toBeUndefined()
    expect(passwordPolicyError('密码密码密码密码')).toBeUndefined()
  })

  it('uses Unicode character count, UTF-8 byte limit, and rejects controls', () => {
    expect(passwordPolicyError('密码密码密码')).toContain('8 个字符')
    expect(passwordPolicyError('valid123\n')).toContain('控制字符')
    const oversized = '密'.repeat(342)
    expect(passwordByteCount(oversized)).toBe(1026)
    expect(passwordPolicyError(oversized)).toContain('1024 字节')
  })
})
