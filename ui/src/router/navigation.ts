import type { RouteLocationNormalized, RouteLocationNormalizedLoaded } from 'vue-router'

type ScrollPosition = { left: number; top: number }

/** One mounted view per page and experiment; sections, drawers and tabs live in the URL without remounting. */
export function routeViewKey(route: RouteLocationNormalizedLoaded, projectId: string | undefined): string {
  const experimentId = typeof route.params.experimentId === 'string' ? route.params.experimentId : ''
  return `${String(route.name ?? route.path)}:${experimentId}:${projectId ?? 'registry'}`
}

export function routeScrollPosition(
  to: RouteLocationNormalized,
  from: RouteLocationNormalized,
  savedPosition: ScrollPosition | null,
): ScrollPosition | { top: number } | false {
  if (savedPosition) return savedPosition
  if (routeViewKey(to, undefined) === routeViewKey(from, undefined)) return false
  return { top: 0 }
}
