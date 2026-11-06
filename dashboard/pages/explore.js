// The store as a shape: a 3D projection you can turn, with exact neighbours one click away.
import { api } from '../api.js'
import { h, fill, icon, button, iconButton, num, select, emptyState, errorNote, errorState, micros, plural } from '../ui.js'
import { assignColors, seriesColors } from '../palette.js'
import { Scatter } from './scatter.js'
import { openVector } from './vector.js'

const SAMPLE_SIZES = [500, 2000, 5000, 10000]

function modelsOf(stats) {
  return [...new Set(stats.groups.map((g) => g.embed_model_id))].filter(Boolean).sort()
}

function namespacesOf(stats) {
  return [...new Set(stats.groups.map((g) => g.namespace))].sort()
}

export function render(ctx, params) {
  const stats = ctx.stats
  if (stats.vectors === 0) {
    return {
      node: h('section', { class: 'panel' }, emptyState({
        mark: true,
        title: 'Nothing to explore yet',
        body: 'Once the store holds vectors, this page shows them as a shape you can turn, with exact neighbours one click away.',
        actions: !stats.read_only && h('a', { class: 'btn primary', href: ctx.url('import'), style: { 'text-decoration': 'none' } }, 'Add vectors'),
      })),
      update: (s) => { if (s.vectors > 0) ctx.rerender() },
    }
  }

  const state = {
    namespace: params.namespace || '',
    model: params.embed_model_id || '',
    limit: SAMPLE_SIZES.includes(Number(params.limit)) ? Number(params.limit) : 2000,
    k: 10,
    points: [],
    selected: null,
    hits: [],
    hidden: new Set(),
    loadedVectors: stats.vectors,
    run: 0,
    pick: 0,
  }
  const slotOf = assignColors(stats.groups)

  const stage = h('div', { class: 'scatter-stage' })
  const caption = h('div', { class: 'scatter-caption', 'aria-live': 'polite' }, 'Loading…')
  const legend = h('div', { class: 'legend' })
  const detail = h('div', { class: 'side-scroll' })
  const changed = h('span')

  const scatter = new Scatter(stage, {
    onSelect: (id) => choose(id),
    tipFor: (p) => [
      h('div', { class: 'truncate mono' }, p.id),
      h('div', { class: 'faint truncate' }, p.namespace),
    ],
  })

  const spin = iconButton(scatter.spinning ? 'pause' : 'play', 'Pause or resume turning', {
    onClick: () => {
      scatter.setSpinning(!scatter.spinning)
      spin.replaceChildren(icon(scatter.spinning ? 'pause' : 'play'))
    },
  })

  function syncUrl() {
    ctx.setParams({ namespace: state.namespace, embed_model_id: state.model, limit: state.limit === 2000 ? '' : state.limit })
  }

  async function load() {
    const run = ++state.run
    caption.textContent = 'Projecting…'
    try {
      const res = await api.projection({ namespace: state.namespace, embed_model_id: state.model, limit: state.limit })
      if (run !== state.run) return
      state.points = res.points.map((p) => ({ ...p, slot: slotOf(p.namespace) }))
      state.loadedVectors = ctx.stats.vectors
      changed.replaceChildren()
      scatter.setPoints(state.points)
      state.selected = null
      state.hits = []
      drawLegend()
      drawDetail()
      const kept = Math.round(res.explained.reduce((a, b) => a + b, 0) * 100)
      caption.textContent = res.total === 0
        ? 'No vectors match these filters.'
        : `A 3D projection (PCA) of ${num(res.points.length)} of ${plural(res.total, 'vector')}. These axes keep ${kept}% of the variance, so distances here are approximate. Neighbours are exact, computed in all ${num(res.dimensions)} dimensions.`
    } catch (e) {
      if (run === state.run) {
        caption.textContent = ''
        caption.append(errorNote(e))
      }
    }
  }

  function drawLegend() {
    const colors = seriesColors()
    const counts = new Map()
    for (const p of state.points) counts.set(p.namespace, (counts.get(p.namespace) || 0) + 1)
    const names = [...counts.keys()].sort((a, b) => counts.get(b) - counts.get(a) || a.localeCompare(b))
    legend.replaceChildren(...names.map((name) => {
      const slot = slotOf(name)
      return h('button', {
        type: 'button',
        'aria-pressed': String(!state.hidden.has(name)),
        title: state.hidden.has(name) ? 'Show this namespace' : 'Hide this namespace',
        onClick: () => {
          if (state.hidden.has(name)) state.hidden.delete(name)
          else state.hidden.add(name)
          scatter.setHidden(state.hidden)
          drawLegend()
        },
      },
      h('span', { class: 'swatch', style: { background: slot >= 0 ? colors.slots[slot] : colors.other } }),
      h('span', { class: 'grow truncate' }, name),
      h('span', { class: 'faint num small' }, num(counts.get(name))))
    }))
  }

  async function choose(id) {
    const pick = ++state.pick
    state.selected = id
    if (id === null) {
      state.hits = []
      scatter.setSelection(null)
      drawDetail()
      return
    }
    state.loading = true
    state.error = null
    scatter.setSelection(id)
    drawDetail()
    try {
      const res = await api.search({ id, k: state.k, namespace: state.namespace, embed_model_id: state.model })
      if (pick !== state.pick) return
      state.hits = res.data
      state.tookMicros = res.took_micros
      scatter.setSelection(id, res.data.map((h) => h.id))
    } catch (e) {
      if (pick === state.pick) state.error = e
    }
    if (pick === state.pick) {
      state.loading = false
      drawDetail()
    }
  }

  function drawDetail() {
    if (state.selected === null) {
      detail.replaceChildren(
        h('div', { class: 'section-label' }, 'Namespaces'),
        legend,
        h('div', { class: 'section-label' }, 'How to use this'),
        h('p', { class: 'small muted', style: { padding: '2px 12px 12px' } }, 'Click a point to find its nearest neighbours. Drag to turn, scroll to zoom. Click a namespace above to hide it.'),
      )
      return
    }
    const here = state.points.find((p) => p.id === state.selected)
    fill(detail,
      h('div', { style: { padding: '10px 12px 8px' } },
        h('div', { class: 'row-gap', style: { 'justify-content': 'space-between', 'flex-wrap': 'nowrap' } },
          h('div', { class: 'mono truncate', title: state.selected, style: { 'font-weight': '550' } }, state.selected),
          iconButton('close', 'Clear the selection', { onClick: () => choose(null) })),
        here && h('div', { class: 'faint small truncate' }, here.namespace),
        h('div', { class: 'row-gap', style: { 'margin-top': '8px' } },
          button('Details', { onClick: () => openVector(ctx, state.selected, { onDeleted: load }) }),
          h('a', { class: 'btn', style: { 'text-decoration': 'none' }, href: ctx.url('search', { id: state.selected }) }, 'Open in Search'))),
      h('div', { class: 'section-label' },
        state.loading ? 'Finding neighbours…' : `${plural(state.hits.length, 'nearest neighbour')}${state.tookMicros ? ` · ${micros(state.tookMicros)}` : ''}`),
      state.error && h('div', { style: { padding: '4px 12px' } }, errorNote(state.error)),
      h('ol', { class: 'hits' }, state.hits.map((hit) => h('li', null, h('button', {
        type: 'button',
        onMouseenter: () => scatter.highlight(hit.id),
        onMouseleave: () => scatter.highlight(null),
        onFocus: () => scatter.highlight(hit.id),
        onBlur: () => scatter.highlight(null),
        onClick: () => choose(hit.id),
      },
      h('span', { class: 'mono truncate small' }, hit.id),
      h('span', { class: 'num small muted' }, hit.score.toFixed(4)),
      h('span', { class: 'faint small truncate', style: { 'grid-column': '1 / -1' } }, hit.namespace))))),
    )
  }

  const models = modelsOf(stats)
  const tools = h('div', { class: 'scatter-tools' },
    h('label', null, 'Namespace', select(namespacesOf(stats), state.namespace, (v) => { state.namespace = v; syncUrl(); load() }, { all: 'All' })),
    new Set(stats.groups.map((g) => g.embed_model_id)).size > 1 && h('label', null, 'Model', select(models, state.model, (v) => { state.model = v; syncUrl(); load() }, { all: 'All' })),
    h('label', null, 'Sample', select(SAMPLE_SIZES.map((s) => ({ value: String(s), label: num(s) })), String(state.limit), (v) => { state.limit = Number(v); syncUrl(); load() })),
    h('span', { class: 'grow' }),
    changed,
    spin,
  )

  const node = h('div', { class: 'explore' },
    h('section', { class: 'panel scatter-panel' }, tools, stage, caption),
    h('section', { class: 'panel side-panel' }, detail))

  return {
    node,
    mounted: () => { scatter.size(); load() },
    update: (s) => {
      if (s.vectors !== state.loadedVectors && !changed.firstChild) {
        changed.append(button('Store changed: refresh', { onClick: load }))
      }
    },
    reloaded: load,
    dispose: () => scatter.dispose(),
  }
}
