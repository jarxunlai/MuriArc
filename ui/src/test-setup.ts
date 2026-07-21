import { config } from '@vue/test-utils'

config.global.stubs = {
  teleport: true,
  transition: false,
}
