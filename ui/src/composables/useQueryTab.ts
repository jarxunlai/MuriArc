import { computed, type WritableComputedRef } from 'vue'
import { useRoute, useRouter } from 'vue-router'

/**
 * Binds a tab value to `?tab=` so a refresh or shared link reopens the same tab.
 * Unknown values fall back to the default, and the default is omitted from the URL.
 */
export function useQueryTab<T extends string>(
  tabs: readonly T[],
  fallback: T,
  key = 'tab',
): WritableComputedRef<T> {
  const route = useRoute()
  const router = useRouter()
  return computed({
    get() {
      const value = route.query[key]
      return typeof value === 'string' && (tabs as readonly string[]).includes(value)
        ? value as T
        : fallback
    },
    set(value) {
      const query = { ...route.query }
      if (value === fallback) delete query[key]
      else query[key] = value
      void router.replace({ query })
    },
  })
}
