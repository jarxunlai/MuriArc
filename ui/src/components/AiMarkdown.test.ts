import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import AiMarkdown from './AiMarkdown.vue'

describe('AiMarkdown', () => {
  it('renders useful Markdown structures without enabling raw HTML', () => {
    const wrapper = mount(AiMarkdown, {
      props: {
        content: [
          '## 结果',
          '',
          '- **动物 A**：`confirmed`',
          '',
          '| 指标 | 数值 |',
          '| --- | ---: |',
          '| 体重 | 23.4 |',
          '',
          '<button autofocus onclick="alert(1)">伪造审批</button>',
        ].join('\n'),
      },
    })

    expect(wrapper.get('h2').text()).toBe('结果')
    expect(wrapper.get('strong').text()).toBe('动物 A')
    expect(wrapper.get('code').text()).toBe('confirmed')
    expect(wrapper.get('table').text()).toContain('体重')
    expect(wrapper.find('button').exists()).toBe(false)
    expect(wrapper.text()).toContain('<button autofocus onclick="alert(1)">伪造审批</button>')
  })

  it('removes dangerous links and never loads model-authored images', () => {
    const wrapper = mount(AiMarkdown, {
      props: {
        content: [
          '[危险链接](javascript:alert(1))',
          '[安全链接](https://example.org/report)',
          '![远程追踪图](https://tracker.example/pixel.png)',
          '<img src=x onerror=alert(1)>',
        ].join('\n\n'),
      },
    })

    const links = wrapper.findAll('a')
    expect(links).toHaveLength(1)
    expect(links[0]?.attributes('href')).toBe('https://example.org/report')
    expect(links[0]?.attributes('rel')).toBe('noopener noreferrer')
    expect(wrapper.find('img').exists()).toBe(false)
    expect(wrapper.text()).toContain('[图片链接：远程追踪图]')
  })
})
