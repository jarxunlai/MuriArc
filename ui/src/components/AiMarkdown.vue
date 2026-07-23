<script setup lang="ts">
import DOMPurify from 'dompurify'
import MarkdownIt from 'markdown-it'
import { computed } from 'vue'

const props = defineProps<{ content: string }>()

const markdown = new MarkdownIt({
  breaks: true,
  html: false,
  linkify: true,
  typographer: false,
})

const fallbackLinkOpen = markdown.renderer.rules.link_open
  ?? ((tokens, index, options, _env, renderer) => renderer.renderToken(tokens, index, options))

markdown.renderer.rules.link_open = (tokens, index, options, env, renderer) => {
  const href = tokens[index]?.attrGet('href') ?? ''
  tokens[index]?.attrSet('rel', 'noopener noreferrer')
  if (/^https?:\/\//i.test(href)) tokens[index]?.attrSet('target', '_blank')
  return fallbackLinkOpen(tokens, index, options, env, renderer)
}

// Model-authored Markdown must not make background network requests or place
// unreviewed visual evidence in the conversation. Uploaded scientific images
// are represented by the separate, trusted source cards.
markdown.renderer.rules.image = (tokens, index) => {
  const label = markdown.utils.escapeHtml(tokens[index]?.content || '图片')
  return `<span class="blocked-markdown-image">[图片链接：${label}]</span>`
}

const html = computed(() => DOMPurify.sanitize(markdown.render(props.content), {
  USE_PROFILES: { html: true },
  FORBID_TAGS: [
    'button', 'embed', 'form', 'iframe', 'input', 'object', 'option',
    'script', 'select', 'style', 'textarea',
  ],
  FORBID_ATTR: ['srcset', 'style'],
  ALLOW_UNKNOWN_PROTOCOLS: false,
}))
</script>

<template>
  <!-- Only assistant content reaches this component. DOMPurify is the final
       boundary even though markdown-it also has raw HTML disabled. -->
  <div class="ai-markdown" v-html="html" />
</template>

<style scoped>
.ai-markdown {
  min-width: 0;
  color: inherit;
  overflow-wrap: anywhere;
}
.ai-markdown :deep(:first-child) { margin-top: 0; }
.ai-markdown :deep(:last-child) { margin-bottom: 0; }
.ai-markdown :deep(p),
.ai-markdown :deep(ul),
.ai-markdown :deep(ol),
.ai-markdown :deep(blockquote),
.ai-markdown :deep(pre),
.ai-markdown :deep(table) { margin: 0 0 0.75em; }
.ai-markdown :deep(ul),
.ai-markdown :deep(ol) { padding-left: 1.45em; }
.ai-markdown :deep(li + li) { margin-top: 0.2em; }
.ai-markdown :deep(h1),
.ai-markdown :deep(h2),
.ai-markdown :deep(h3),
.ai-markdown :deep(h4) {
  margin: 1em 0 0.45em;
  color: var(--muri-text);
  font-weight: 650;
  line-height: 1.3;
}
.ai-markdown :deep(h1) { font-size: 1.28em; }
.ai-markdown :deep(h2) { font-size: 1.17em; }
.ai-markdown :deep(h3),
.ai-markdown :deep(h4) { font-size: 1.06em; }
.ai-markdown :deep(a) {
  color: var(--muri-primary);
  text-decoration: underline;
  text-decoration-thickness: 1px;
  text-underline-offset: 2px;
}
.ai-markdown :deep(a:focus-visible) {
  border-radius: 2px;
  outline: 2px solid var(--muri-primary);
  outline-offset: 2px;
}
.ai-markdown :deep(blockquote) {
  padding: 0.15em 0 0.15em 0.8em;
  border-left: 3px solid var(--muri-border-strong);
  color: var(--muri-text-secondary);
}
.ai-markdown :deep(code) {
  padding: 0.12em 0.32em;
  border-radius: 4px;
  color: #26425d;
  background: #edf2f6;
  font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
  font-size: 0.9em;
}
.ai-markdown :deep(pre) {
  max-width: 100%;
  padding: 10px 12px;
  border: 1px solid var(--muri-border);
  border-radius: 7px;
  background: #f4f7f9;
  overflow: auto;
}
.ai-markdown :deep(pre code) {
  padding: 0;
  color: var(--muri-text);
  background: transparent;
  white-space: pre;
}
.ai-markdown :deep(table) {
  display: block;
  width: max-content;
  max-width: 100%;
  border-collapse: collapse;
  overflow-x: auto;
}
.ai-markdown :deep(th),
.ai-markdown :deep(td) {
  padding: 6px 9px;
  border: 1px solid var(--muri-border);
  text-align: left;
  white-space: nowrap;
}
.ai-markdown :deep(th) { background: var(--muri-surface-muted); }
.ai-markdown :deep(.blocked-markdown-image) {
  color: var(--muri-text-tertiary);
  font-size: 0.9em;
}
</style>
