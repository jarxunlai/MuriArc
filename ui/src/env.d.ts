/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_MURIARC_GATEWAY?: 'local' | 'remote' | 'demo'
  readonly VITE_MURIARC_API_BASE?: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
