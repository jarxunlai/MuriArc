import { config } from '@vue/test-utils'

config.global.stubs = {
  teleport: true,
  transition: false,
}
// Naive UI components are imported locally by each SFC; render the default slot
// of shallow stubs so view tests can still assert on the content they wrap.
config.global.renderStubDefaultSlot = true
