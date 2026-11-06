// Add vectors: paste or drop a file, check it before anything is written, then send it in
// batches. A failure never loses the input, and retrying is safe because adds overwrite by id.
import { api } from '../api.js'
import { h, field, button, num, plural, bytes, toast, confirmDialog, errorNote, readOnlyNote } from '../ui.js'

const BATCH = 500
const EXAMPLE = `[
  { "id": "doc-1", "vector": [0.12, -0.03, 0.88], "metadata": { "title": "Hello" } },
  { "id": "doc-2", "vector": [0.70, 0.11, -0.25], "namespace": "notes" }
]`
const SAMPLE_PER_CLUSTER = 250

let sampleRequested = false

/** Load the sample when this page next opens. Not a URL parameter: a link must never write to the store. */
export function requestSample() {
  sampleRequested = true
}

/** Accepts a JSON array, {"vectors": [...]}, or one JSON object per line. */
export function parseInput(text) {
  const trimmed = text.trim()
  if (!trimmed) return { rows: [], errors: [] }
  let rows
  try {
    const parsed = JSON.parse(trimmed)
    rows = Array.isArray(parsed) ? parsed : Array.isArray(parsed?.vectors) ? parsed.vectors : [parsed]
  } catch {
    rows = []
    const errors = []
    trimmed.split('\n').forEach((line, i) => {
      if (!line.trim()) return
      try {
        rows.push(JSON.parse(line))
      } catch {
        errors.push(`Line ${i + 1} isn't valid JSON.`)
      }
    })
    if (errors.length) return { rows: [], errors }
  }
  return { rows, errors: [] }
}

export function validate(rows, storeDimensions) {
  const errors = []
  let dimensions = storeDimensions
  rows.forEach((r, i) => {
    if (errors.length >= 5) return
    const label = `Row ${i + 1}`
    if (typeof r !== 'object' || r === null) errors.push(`${label} isn't an object.`)
    else if (typeof r.id !== 'string' || !r.id) errors.push(`${label} needs a string "id".`)
    else if (!Array.isArray(r.vector) || r.vector.length === 0 || !r.vector.every((x) => typeof x === 'number')) errors.push(`${label} ("${r.id}") needs a "vector": an array of numbers.`)
    else if (dimensions && r.vector.length !== dimensions) errors.push(`${label} ("${r.id}") has ${num(r.vector.length)} numbers; expected ${num(dimensions)}.`)
    else dimensions = r.vector.length
  })
  return { errors, dimensions }
}

/** Clusters of random vectors, the same every time. Plainly labelled as synthetic. */
function sampleVectors(dimensions) {
  let seed = 7
  const random = () => {
    seed = (seed + 0x6d2b79f5) | 0
    let x = Math.imul(seed ^ (seed >>> 15), 1 | seed)
    x = (x + Math.imul(x ^ (x >>> 7), 61 | x)) ^ x
    return ((x ^ (x >>> 14)) >>> 0) / 4294967296
  }
  const gauss = () => Math.sqrt(-2 * Math.log(random() + 1e-9)) * Math.cos(2 * Math.PI * random())
  const namespaces = ['sample-docs', 'sample-docs', 'sample-code', 'sample-code', 'sample-wiki', 'sample-wiki']
  const rows = []
  namespaces.forEach((namespace, cluster) => {
    const centre = Array.from({ length: dimensions }, gauss)
    const norm = Math.sqrt(centre.reduce((a, x) => a + x * x, 0))
    for (let i = 0; i < SAMPLE_PER_CLUSTER; i++) {
      rows.push({
        id: `${namespace}-${cluster}-${String(i).padStart(3, '0')}`,
        vector: centre.map((c) => Number((c / norm + gauss() * 0.42 / Math.sqrt(dimensions)).toFixed(5))),
        namespace,
        embed_model_id: 'synthetic',
        metadata: { synthetic: true, cluster },
      })
    }
  })
  return rows
}

export function render(ctx) {
  const stats = ctx.stats
  if (stats.read_only) {
    return { node: h('section', { class: 'panel' }, h('h2', null, 'Import'), h('div', { class: 'panel-body' }, readOnlyNote(stats, "importing isn't available"))) }
  }

  const state = { file: null, rows: [], busy: false }
  const preview = h('div', { class: 'stack' })
  const feedback = h('div', { class: 'stack' })
  const namespaceInput = h('input', { type: 'text', placeholder: 'default', spellcheck: 'false' })
  const modelInput = h('input', { type: 'text', placeholder: 'untagged', spellcheck: 'false' })
  const textarea = h('textarea', {
    class: 'mono', rows: 13, spellcheck: 'false', placeholder: EXAMPLE, 'aria-label': 'Vectors as JSON',
    onInput: () => { state.file = null; check() },
  })
  const fileInput = h('input', {
    type: 'file', accept: '.json,.jsonl,.ndjson,application/json', class: 'sr-only', id: 'import-file',
    onChange: (e) => { if (e.target.files[0]) readFile(e.target.files[0]) },
  })
  const addButton = button('Add vectors', { kind: 'primary', disabled: true, onClick: () => send(state.rows) })
  const fileNote = h('span', { class: 'small faint' })

  async function readFile(file) {
    state.file = file
    textarea.value = ''
    fileNote.textContent = `${file.name} · ${bytes(file.size)}`
    feedback.replaceChildren()
    check(await file.text())
  }

  function check(text = textarea.value) {
    feedback.replaceChildren()
    const { rows, errors } = parseInput(text)
    const checked = errors.length ? { errors } : validate(rows, ctx.stats.dimensions)
    state.rows = checked.errors.length ? [] : rows
    addButton.disabled = state.rows.length === 0 || state.busy
    addButton.textContent = state.rows.length ? `Add ${plural(state.rows.length, 'vector')}` : 'Add vectors'
    if (!state.file) fileNote.textContent = ''
    if (checked.errors.length) {
      preview.replaceChildren(errorNote(new Error(checked.errors.join(' '))))
    } else if (rows.length) {
      const namespaces = new Set(rows.map((r) => r.namespace).filter(Boolean))
      preview.replaceChildren(h('dl', { class: 'facts' },
        h('dt', null, 'Vectors'), h('dd', null, num(rows.length)),
        h('dt', null, 'Dimensions'), h('dd', null, num(checked.dimensions)),
        h('dt', null, 'Namespaces in the file'), h('dd', { class: 'truncate' }, namespaces.size ? [...namespaces].slice(0, 3).join(', ') + (namespaces.size > 3 ? ` +${namespaces.size - 3}` : '') : 'None')))
    } else {
      preview.replaceChildren(h('p', { class: 'muted' }, 'Paste vectors or choose a file, and they\'ll be checked here before anything is written.'))
    }
  }

  async function send(rows, { thenExplore = false } = {}) {
    if (state.busy || rows.length === 0) return
    state.busy = true
    addButton.disabled = true
    sampleButton.disabled = true
    const namespace = namespaceInput.value.trim()
    const model = modelInput.value.trim()
    const bar = h('i', { style: { width: '0%' } })
    const status = h('span', { class: 'small muted', 'aria-live': 'polite' }, `Adding 0 of ${num(rows.length)}…`)
    feedback.replaceChildren(h('div', { class: 'progress', role: 'progressbar', 'aria-label': 'Import progress' }, bar), status)
    let sent = 0
    try {
      for (let i = 0; i < rows.length; i += BATCH) {
        const batch = rows.slice(i, i + BATCH).map((r) => ({
          ...r,
          namespace: r.namespace || namespace || undefined,
          embed_model_id: r.embed_model_id || model || undefined,
        }))
        await api.add(batch)
        sent += batch.length
        bar.style.setProperty('width', `${(sent / rows.length) * 100}%`)
        status.textContent = `Adding ${num(sent)} of ${num(rows.length)}…`
      }
      toast(`Added ${plural(sent, 'vector')}`)
      await ctx.refresh()
      feedback.replaceChildren()
      if (thenExplore) return ctx.navigate('explore')
      textarea.value = ''
      fileInput.value = ''
      state.file = null
      check()
      textarea.focus()
    } catch (e) {
      await ctx.refresh()
      // The input stays where it is. Adds overwrite by id, so running it again is safe.
      feedback.replaceChildren(errorNote(new Error(`${e.message} ${num(sent)} of ${num(rows.length)} were added first. Your input is still here, and adding it again is safe: existing ids are overwritten.`)))
    } finally {
      state.busy = false
      sampleButton.disabled = false
      addButton.disabled = state.rows.length === 0
    }
  }

  async function loadSample() {
    const dimensions = ctx.stats.dimensions || 64
    const rows = sampleVectors(dimensions)
    if (ctx.stats.vectors > 0) {
      const yes = await confirmDialog({
        title: 'Add sample data to this store?',
        body: `This adds ${num(rows.length)} synthetic vectors to the store, in namespaces starting with "sample-". You can delete them later.`,
        confirm: 'Add sample data',
      })
      if (!yes) return
    }
    send(rows, { thenExplore: true })
  }
  const sampleButton = button('Load sample data', { onClick: loadSample })

  const drop = h('div', {
    class: 'drop',
    onDragover: (e) => { e.preventDefault(); drop.classList.add('over') },
    onDragleave: () => drop.classList.remove('over'),
    onDrop: (e) => {
      e.preventDefault()
      drop.classList.remove('over')
      if (e.dataTransfer.files[0]) readFile(e.dataTransfer.files[0])
    },
  },
  textarea,
  h('div', { class: 'row-gap' },
    fileInput,
    h('label', { class: 'btn', for: 'import-file', tabindex: '0', onKeydown: (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); fileInput.click() } } }, 'Choose a file'),
    fileNote,
    h('span', { class: 'small faint' }, 'or drop one here. A JSON array, or one object per line.')),
  h('p', { class: 'small faint' }, 'Each row needs "id" and "vector". "metadata", "namespace" and "embed_model_id" are optional.'))

  check()

  const node = h('div', { class: 'import-grid' },
    h('section', { class: 'panel' },
      h('h2', null, 'Vectors'),
      h('div', { class: 'panel-body' }, drop)),
    h('div', { class: 'stack' },
      h('section', { class: 'panel' },
        h('h2', null, 'Check'),
        h('div', { class: 'panel-body stack' }, preview,
          h('div', { class: 'two' },
            field('Default namespace', namespaceInput),
            field('Default model', modelInput)),
          h('p', { class: 'small faint' }, 'These apply to rows that don\'t set their own. Tag your embedding model so searches can filter by it.'),
          addButton,
          feedback)),
      h('section', { class: 'panel' },
        h('h2', null, 'Just trying it out?'),
        h('div', { class: 'panel-body stack' },
          h('p', { class: 'muted' }, `Load ${num(SAMPLE_PER_CLUSTER * 6)} synthetic vectors, grouped into clusters, to see the Explore and Search pages at work.`),
          h('div', null, sampleButton)))))

  return {
    node,
    mounted: () => {
      if (sampleRequested) {
        sampleRequested = false
        loadSample()
      }
    },
  }
}
