import { createApp } from 'vue'
import App from './App.vue'
import { router } from './router'
import './styles/tokens.css'
import './styles/global.css'

// Naive UI components are imported by each SFC that uses them, so route chunks
// only ship the controls they render (DataTable, DatePicker, ...) instead of the
// whole set landing in the entry bundle.
createApp(App).use(router).mount('#app')
