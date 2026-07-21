import brand from '../../branding/brand.json'

export interface BrandingConfig {
  productName: string
  shortName: string
  tagline: string
  bundleIdentifier: string
  primaryColor: string
  accentColor: string
  sourceNotice: string
  logoMarkPath: string
  version: string
  releaseStage: string
  logoMasterSha256: string
}

// Keep the public shape explicit so every UI surface consumes the same
// centrally-versioned contract instead of inferring a stale generated JSON type.
export const branding: Readonly<BrandingConfig> = Object.freeze(brand)
