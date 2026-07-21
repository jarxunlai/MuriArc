export const LOCAL_WELCOME_SESSION_KEY = 'muriarc.local-space.entered.v1'

export function hasEnteredLocalSpace(): boolean {
  try {
    return sessionStorage.getItem(LOCAL_WELCOME_SESSION_KEY) === 'true'
  } catch {
    return false
  }
}

export function markLocalSpaceEntered(): void {
  try {
    sessionStorage.setItem(LOCAL_WELCOME_SESSION_KEY, 'true')
  } catch {
    // Hardened WebViews may disable sessionStorage. Navigation still works,
    // but a refresh can show the non-security welcome page again.
  }
}
