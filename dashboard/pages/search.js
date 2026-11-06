// Run a query and read the results: by a stored vector's id, or by a vector you paste.
import { api } from '../api.js'
import { h, field, select, tabs, button, num, micros, plural, emptyState, errorNote, skeleton, metadataPreview, shellQuote, toast } from '../ui.js'
import { openVector } from './vector.js'

export function render(ctx, params) {
  const stats = ctx.stats
  const state = {
    source: params.vector ? 'paste' : 'id',
    k: Number(params.k) || 10,
    namespace: params.namespace || '',
    model: params.embed_model_id || '',
    busy: false,
    run: 0,
    lastBody: null,
  }
  const results = h('div')
  const suggestions = h('datalist', { id: 'id-suggestions' })
  let lookup = 0

  const idInput = h('input', {
    type: 'text', class: 'mono', list: 'id-suggestions', placeholder: 'Type to find a vector', value: params.id || '',
    autocomplete: 'off', spellcheck: 'false',
    onInput: async (e) => {
      const q = e.target.value
      const mine = ++lookup
      if (q.length < 1) return
      try {
        const { data } = await api.list({ q, limit: 8 })
        if (mine === lookup) suggestions.replaceChildren(...data.map((v) => h('option', { value: v.id })))
      } catch { /* suggestions are optional */ }
    },
    onKeydown: (e) => { if (e.key === 'Enter') run() },
  })
  const vectorHint = h('span', { class: 'small faint' })
  const vectorInput = h('textarea', {
    class: 'mono', rows: 3, spellcheck: 'false', placeholder: stats.dimensions ? `[0.12, -0.03, …] (${num(stats.dimensions)} numbers)` : '[0.12, -0.03, …]',
    value: params.vector || '',
    onInput: hint,
    onKeydown: (e) => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) run() },
  })

  function parseVector() {
    let parsed
    try {
      parsed = JSON.parse(vectorInput.value)
    } catch {
      return { error: 'Enter a JSON array of numbers.' }
    }
    if (!Array.isArray(parsed) || parsed.length === 0 || !parsed.every((x) => typeof x === 'number')) {
      return { error: 'Enter a JSON array of numbers.' }
    }
    if (stats.dimensions && parsed.length !== stats.dimensions) {
      return { error: `That's ${num(parsed.length)} numbers. This store holds ${num(stats.dimensions)}-dimensional vectors.` }
    }
    return { vector: parsed }
  }

  function hint() {
    if (!vectorInput.value.trim()) {
      vectorHint.textContent = ''
      return
    }
    const { error, vector } = parseVector()
    vectorHint.textContent = error || `${num(vector.length)} numbers: ready`
  }

  const sourceBox = h('div')
  function drawSource() {
    sourceBox.replaceChildren(state.source === 'id'
      ? field('Stored vector', idInput)
      : h('div', { class: 'field' }, h('label', { class: 'field' }, h('span', null, 'Query vector'), vectorInput), vectorHint))
  }

  const namespaces = [...new Set(stats.groups.map((g) => g.namespace))].sort()
  const models = [...new Set(stats.groups.map((g) => g.embed_model_id))].filter(Boolean).sort()
  const kInput = h('input', {
    type: 'number', min: 1, max: 1000, value: String(state.k), class: 'num',
    onInput: (e) => { state.k = Number(e.target.value) },
    onKeydown: (e) => { if (e.key === 'Enter') run() },
  })
  const runButton = button('Search', { kind: 'primary', onClick: run })

  function buildBody() {
    const body = { k: state.k, namespace: state.namespace || undefined, embed_model_id: state.model || undefined }
    if (state.source === 'id') {
      if (!idInput.value.trim()) return { error: 'Enter the id of a stored vector.' }
      body.id = idInput.value.trim()
    } else {
      const { error, vector } = parseVector()
      if (error) return { error }
      body.vector = vector
    }
    return { body }
  }

  async function run() {
    const { error, body } = buildBody()
    if (error) {
      results.replaceChildren(errorNote(new Error(error)))
      return
    }
    const mine = ++state.run
    runButton.disabled = true
    results.replaceChildren(h('section', { class: 'panel' }, skeleton(6)))
    ctx.setParams({
      id: body.id,
      k: state.k === 10 ? '' : state.k,
      namespace: state.namespace,
      embed_model_id: state.model,
    })
    try {
      const res = await api.search(body)
      if (mine !== state.run) return
      state.lastBody = body
      drawResults(res)
      ctx.refresh()
    } catch (e) {
      if (mine === state.run) results.replaceChildren(errorNote(e))
    } finally {
      if (mine === state.run) runButton.disabled = false
    }
  }

  function asCurl(body) {
    return `curl -s ${location.origin}/api/search -H 'Content-Type: application/json' -d ${shellQuote(JSON.stringify(body))}`
  }

  function drawResults(res) {
    const summary = h('div', { class: 'panel-head' },
      h('span', { style: { 'font-weight': '500' } },
        `${plural(res.data.length, 'hit')} in ${micros(res.took_micros)}`,
        h('span', { class: 'faint', style: { 'font-weight': '400' } }, ` · compared against ${plural(res.searched, 'vector')}, exact`)),
      button('Copy as curl', {
        iconName: 'copy',
        onClick: async () => {
          try {
            await navigator.clipboard.writeText(asCurl(state.lastBody))
            toast('Copied')
          } catch {
            toast("Couldn't copy: the browser blocked it.", 'error')
          }
        },
      }))
    if (res.data.length === 0) {
      results.replaceChildren(h('section', { class: 'panel' }, summary,
        emptyState({ title: 'No hits', body: 'Nothing else matches these filters. Try clearing the namespace or model.' })))
      return
    }
    results.replaceChildren(h('section', { class: 'panel' }, summary,
      h('div', { class: 'table-wrap' }, h('table', null,
        h('thead', null, h('tr', null,
          h('th', { scope: 'col', class: 'right' }, '#'),
          h('th', { scope: 'col' }, 'ID'),
          h('th', { scope: 'col' }, 'Cosine score'),
          h('th', { scope: 'col' }, 'Namespace'),
          h('th', { scope: 'col' }, 'Model'),
          h('th', { scope: 'col' }, 'Metadata'))),
        h('tbody', null, res.data.map((hit, i) => h('tr', { class: 'row' },
          h('td', { class: 'right num faint' }, String(i + 1)),
          h('td', { style: { 'max-width': '260px' } }, h('button', {
            type: 'button', class: 'linkish mono truncate', title: hit.id, onClick: () => openVector(ctx, hit.id, { onDeleted: run }),
          }, hit.id)),
          h('td', null, h('span', { class: 'score' },
            h('span', { class: 'bar lit', 'aria-hidden': 'true' }, h('i', { style: { width: `${Math.max(0, hit.score) * 100}%` } })),
            h('span', { class: 'num' }, hit.score.toFixed(4)))),
          h('td', { style: { 'max-width': '160px' } }, h('span', { class: 'tag' }, hit.namespace)),
          h('td', { style: { 'max-width': '160px' } }, h('span', { class: 'tag' }, hit.embed_model_id || 'untagged')),
          h('td', { class: 'wide truncate mono small faint' }, metadataPreview(hit.metadata)))))))))
  }

  drawSource()

  const panel = h('section', { class: 'panel query-panel', style: { 'margin-bottom': '12px' } },
    h('div', { class: 'panel-body' },
      tabs([{ value: 'id', label: 'A stored vector' }, { value: 'paste', label: 'Paste a vector' }], state.source, (v) => {
        state.source = v
        drawSource()
      }, 'What to search with'),
      h('div', { class: 'search-form' },
        sourceBox,
        field('Top k', kInput),
        field('Namespace', select(namespaces, state.namespace, (v) => { state.namespace = v }, { all: 'All' })),
        field('Model', select(models, state.model, (v) => { state.model = v }, { all: 'All' })),
        runButton)),
    suggestions)

  if (stats.vectors === 0) {
    results.replaceChildren(h('section', { class: 'panel' }, emptyState({ title: 'The store is empty', body: 'Add some vectors first, then search them here.' })))
  }

  return {
    node: h('div', null, panel, results),
    update: (s) => { if (stats.vectors === 0 && s.vectors > 0) ctx.rerender() },
    mounted: () => {
      if (params.id) run()
      else if (state.source === 'id') idInput.focus({ preventScroll: true })
    },
  }
}
