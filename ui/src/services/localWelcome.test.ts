import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  LOCAL_WELCOME_SESSION_KEY,
  hasEnteredLocalSpace,
  markLocalSpaceEntered,
} from './localWelcome'

describe('local welcome session marker', () => {
  beforeEach(() => sessionStorage.clear())

  it('persists only in sessionStorage for the current WebView session', () => {
    expect(hasEnteredLocalSpace()).toBe(false)
    markLocalSpaceEntered()
    expect(sessionStorage.getItem(LOCAL_WELCOME_SESSION_KEY)).toBe('true')
    expect(hasEnteredLocalSpace()).toBe(true)
    expect(localStorage.getItem(LOCAL_WELCOME_SESSION_KEY)).toBeNull()
  })

  it('fails open for entry while reporting not-entered when storage is unavailable', () => {
    const get = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new DOMException('blocked')
    })
    const set = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('blocked')
    })
    expect(hasEnteredLocalSpace()).toBe(false)
    expect(() => markLocalSpaceEntered()).not.toThrow()
    get.mockRestore()
    set.mockRestore()
  })
})
