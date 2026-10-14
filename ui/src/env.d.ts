/// <reference types="vite/client" />

/** True in dev/test and in builds that select `VITE_MURIARC_GATEWAY=demo`. */
declare const __MURIARC_DEMO_GATEWAY__: boolean

interface ImportMetaEnv {
  readonly VITE_MURIARC_GATEWAY?: 'local' | 'remote' | 'demo'
  readonly VITE_MURIARC_API_BASE?: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
