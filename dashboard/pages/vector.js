// One vector, in a drawer: its metadata, its values, and what you can do with it.
import { api } from '../api.js'
import { h, fill, num, button, copyButton, openDrawer, confirmDialog, toast, fingerprint, skeleton, errorNote } from '../ui.js'

function fact(label, value) {
  return [h('dt', null, label), h('dd', { class: 'truncate', title: value }, value)]
}

export function openVector(ctx, id, { onDeleted } = {}) {
  const body = h('div', { class: 'stack' }, skeleton(5))
  const foot = h('div', { class: 'row-gap', style: { width: '100%' } })
  const dialog = openDrawer(id, body, foot, copyButton(() => id, 'Copy id'))

  api.get(id).then((v) => {
    const metadata = JSON.stringify(v.metadata, null, 2)
    const norm = Math.sqrt(v.vector.reduce((a, x) => a + x * x, 0))
    const asJson = () => JSON.stringify(v.vector)
    body.replaceChildren(
      h('dl', { class: 'facts' },
        fact('Namespace', v.namespace),
        fact('Embedding model', v.embed_model_id || 'untagged'),
        fact('Dimensions', num(v.vector.length)),
        fact('Length (L2 norm)', norm.toFixed(4))),
      h('section', { class: 'stack', style: { gap: '6px' } },
        h('div', { class: 'row-gap', style: { 'justify-content': 'space-between' } },
          h('h3', { class: 'small muted', style: { 'font-weight': '500' } }, 'Metadata'),
          v.metadata !== null && copyButton(() => metadata, 'Copy metadata')),
        v.metadata === null
          ? h('p', { class: 'faint' }, 'None')
          : h('div', { class: 'code wrap' }, h('pre', { tabindex: '0' }, h('code', null, metadata)))),
      h('section', { class: 'stack', style: { gap: '6px' } },
        h('div', { class: 'row-gap', style: { 'justify-content': 'space-between' } },
          h('h3', { class: 'small muted', style: { 'font-weight': '500' } }, 'Values'),
          copyButton(asJson, 'Copy all values as JSON')),
        fingerprint(v.vector),
        h('p', { class: 'mono small faint', style: { 'overflow-wrap': 'anywhere' } },
          `[${v.vector.slice(0, 8).map((x) => x.toFixed(4)).join(', ')}${v.vector.length > 8 ? `, … ${num(v.vector.length - 8)} more` : ''}]`)),
    )
    fill(foot,
      h('a', {
        class: 'btn primary',
        style: { 'text-decoration': 'none' },
        href: ctx.url('search', { id }),
        onClick: () => dialog.close(),
      }, 'Find similar'),
      h('span', { class: 'grow' }),
      !ctx.stats.read_only && button('Delete', {
        kind: 'danger',
        iconName: 'trash',
        onClick: async () => {
          const yes = await confirmDialog({
            title: 'Delete this vector?',
            body: `"${id}" will be removed from the store. This can't be undone.`,
            confirm: 'Delete',
            danger: true,
          })
          if (!yes) return
          try {
            await api.remove(id)
            dialog.close()
            toast('Vector deleted')
            await ctx.refresh()
            onDeleted?.()
            document.getElementById('main').focus({ preventScroll: true })
          } catch (e) {
            toast(e.message, 'error')
          }
        },
      }),
    )
  }).catch((e) => body.replaceChildren(errorNote(e)))
}
