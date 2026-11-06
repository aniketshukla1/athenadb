// Every vector, as a table: filter it, page through it, open one.
import { api } from '../api.js'
import { h, field, select, button, num, emptyState, errorState, skeleton, metadataPreview } from '../ui.js'
import { openVector } from './vector.js'

const PAGE_SIZES = [25, 50, 100, 250]

export function render(ctx, params) {
  const state = {
    q: params.q || '',
    namespace: params.namespace || '',
    model: params.embed_model_id || '',
    limit: PAGE_SIZES.includes(Number(params.limit)) ? Number(params.limit) : 50,
    offset: Math.max(0, Number(params.offset) || 0),
    run: 0,
    seenVectors: ctx.stats.vectors,
  }
  const body = h('section', { class: 'panel' }, skeleton(10))
  let debounce = 0

  function sync() {
    ctx.setParams({
      q: state.q,
      namespace: state.namespace,
      embed_model_id: state.model,
      limit: state.limit === 50 ? '' : state.limit,
      offset: state.offset || '',
    })
  }

  async function load() {
    const mine = ++state.run
    sync()
    try {
      const res = await api.list({
        q: state.q, namespace: state.namespace, embed_model_id: state.model, limit: state.limit, offset: state.offset,
      })
      if (mine !== state.run) return
      // Deleting the last row of the last page leaves an empty page: step back.
      if (res.data.length === 0 && state.offset > 0 && res.pagination.total > 0) {
        state.offset = Math.max(0, res.pagination.total - state.limit)
        return load()
      }
      draw(res)
    } catch (e) {
      if (mine === state.run) body.replaceChildren(errorState(e, load))
    }
  }

  function clearFilters() {
    state.q = ''
    state.namespace = ''
    state.model = ''
    state.offset = 0
    qInput.value = ''
    drawFilters()
    load()
  }

  function draw(res) {
    const { total } = res.pagination
    if (total === 0) {
      const filtered = state.q || state.namespace || state.model
      body.replaceChildren(filtered
        ? emptyState({ title: 'No vectors match', body: 'Try fewer filters, or a different part of the id.', actions: button('Clear filters', { onClick: clearFilters }) })
        : emptyState({
          mark: true,
          title: 'This store is empty',
          body: 'Vectors will be listed here once they exist.',
          actions: !ctx.stats.read_only && h('a', { class: 'btn primary', href: ctx.url('import'), style: { 'text-decoration': 'none' } }, 'Add vectors'),
        }))
      return
    }
    const from = state.offset + 1
    const to = Math.min(state.offset + state.limit, total)
    body.replaceChildren(
      h('div', { class: 'table-wrap' }, h('table', null,
        h('thead', null, h('tr', null,
          h('th', { scope: 'col' }, 'ID'),
          h('th', { scope: 'col' }, 'Namespace'),
          h('th', { scope: 'col' }, 'Model'),
          h('th', { scope: 'col' }, 'Metadata'))),
        h('tbody', null, res.data.map((v) => h('tr', { class: 'row' },
          h('td', { style: { 'max-width': '300px' } }, h('button', {
            type: 'button', class: 'linkish mono truncate', title: v.id, onClick: () => openVector(ctx, v.id, { onDeleted: load }),
          }, v.id)),
          h('td', { style: { 'max-width': '170px' } }, h('span', { class: 'tag' }, v.namespace)),
          h('td', { style: { 'max-width': '170px' } }, h('span', { class: 'tag' }, v.embed_model_id || 'untagged')),
          h('td', { class: 'wide truncate mono small faint' }, metadataPreview(v.metadata))))))),
      h('div', { class: 'pager' },
        h('span', { class: 'num' }, `${num(from)}–${num(to)} of ${num(total)}`),
        h('div', { class: 'row-gap' },
          button('Previous', { disabled: state.offset === 0, onClick: () => { state.offset = Math.max(0, state.offset - state.limit); load() } }),
          button('Next', { disabled: to >= total, onClick: () => { state.offset += state.limit; load() } }))),
    )
  }

  const qInput = h('input', {
    type: 'search', placeholder: 'Part of an id', value: state.q, class: 'mono', spellcheck: 'false',
    onInput: (e) => {
      state.q = e.target.value
      state.offset = 0
      clearTimeout(debounce)
      debounce = setTimeout(load, 200)
    },
  })

  const filters = h('div', { class: 'toolbar' })
  function drawFilters() {
    const stats = ctx.stats
    const namespaces = [...new Set(stats.groups.map((g) => g.namespace))].sort()
    const models = [...new Set(stats.groups.map((g) => g.embed_model_id))].filter(Boolean).sort()
    filters.replaceChildren(
      h('div', { style: { flex: '1 1 220px' } }, field('Search ids', qInput)),
      h('div', { style: { flex: '0 1 170px' } }, field('Namespace', select(namespaces, state.namespace, (v) => { state.namespace = v; state.offset = 0; load() }, { all: 'All' }))),
      h('div', { style: { flex: '0 1 170px' } }, field('Model', select(models, state.model, (v) => { state.model = v; state.offset = 0; load() }, { all: 'All' }))),
      h('div', { style: { flex: '0 0 92px' } }, field('Per page', select(PAGE_SIZES.map((n) => ({ value: String(n), label: String(n) })), String(state.limit), (v) => { state.limit = Number(v); state.offset = 0; load() }))),
    )
  }
  drawFilters()

  return {
    node: h('div', null, filters, body),
    mounted: load,
    update: (s) => {
      if (s.vectors !== state.seenVectors) {
        state.seenVectors = s.vectors
        load()
      }
    },
    reloaded: load,
    dispose: () => clearTimeout(debounce),
  }
}
