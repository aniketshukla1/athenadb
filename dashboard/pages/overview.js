// What's in the store, and is it healthy?
import { api } from '../api.js'
import { h, fill, icon, button, num, bytes, micros, ago, fullDate, plural, emptyState, toast, shellQuote, codeBlock } from '../ui.js'
import { requestSample } from './import.js'

function kpi(label, value, sub) {
  return h('div', { class: 'kpi' }, h('dt', null, label), h('dd', null, value), sub && h('dd', { class: 'sub' }, sub))
}

function namespaces(groups) {
  const byName = new Map()
  for (const g of groups) {
    const row = byName.get(g.namespace) || { namespace: g.namespace, vectors: 0, models: [] }
    row.vectors += g.vectors
    row.models.push(g.embed_model_id)
    byName.set(g.namespace, row)
  }
  return [...byName.values()].sort((a, b) => b.vectors - a.vectors || a.namespace.localeCompare(b.namespace))
}

function namespacePanel(s, ctx) {
  const rows = namespaces(s.groups)
  return h('section', { class: 'panel' },
    h('h2', null, 'Namespaces'),
    h('div', { class: 'table-wrap' }, h('table', null,
      h('thead', null, h('tr', null,
        h('th', { scope: 'col' }, 'Namespace'),
        h('th', { scope: 'col' }, 'Embedding model'),
        h('th', { scope: 'col', class: 'right' }, 'Vectors'),
        h('th', { scope: 'col' }, h('span', { class: 'sr-only' }, 'Share of the store')))),
      h('tbody', null, rows.map((r) => {
        const share = s.vectors ? r.vectors / s.vectors : 0
        return h('tr', { class: 'row' },
          h('th', { scope: 'row', class: 'truncate', style: { 'font-weight': '400', 'max-width': '220px', color: 'var(--fg)', 'font-size': '13px' } },
            h('a', { href: ctx.url('browse', { namespace: r.namespace }), style: { color: 'inherit' } }, r.namespace)),
          h('td', null, r.models.map((m) => h('span', { class: 'tag', style: { 'margin-right': '4px', 'max-width': '180px' } }, m || 'untagged'))),
          h('td', { class: 'right num' }, num(r.vectors)),
          h('td', { style: { width: '130px' } },
            h('span', { class: 'bar', title: `${Math.round(share * 100)}% of the store`, style: { width: '78px' } },
              h('i', { style: { width: `${Math.max(share * 100, 1)}%` } })),
            h('span', { class: 'small faint num', style: { 'margin-left': '8px' } }, `${Math.round(share * 100)}%`)))
      })))))
}

function durabilityPanel(s, ctx) {
  const d = s.durability
  const progress = d.snapshot_interval > 0 ? Math.min(1, d.ops_since_snapshot / d.snapshot_interval) : 0
  let busy = false
  const snapshotNow = button('Snapshot now', {
    // Not disabled while it runs: that would drop keyboard focus.
    onClick: async () => {
      if (busy) return
      busy = true
      try {
        await api.snapshot()
        toast('Snapshot written. The log is empty again.')
        await ctx.refresh()
      } catch (err) {
        toast(err.message, 'error')
      } finally {
        busy = false
      }
    },
  })
  return h('section', { class: 'panel' },
    h('div', { class: 'panel-head' }, 'Durability', !s.read_only && snapshotNow),
    h('div', { class: 'panel-body' },
      h('dl', { class: 'facts' },
        h('dt', null, 'Write-ahead log'), h('dd', null, `${bytes(d.wal_bytes)} · ${plural(d.ops_since_snapshot, 'operation')}`),
        h('dt', null, 'Last snapshot'), h('dd', { title: d.snapshot_modified_unix ? fullDate(d.snapshot_modified_unix) : undefined },
          d.snapshot_modified_unix ? `${ago(d.snapshot_modified_unix)} · ${bytes(d.snapshot_bytes)}` : 'None yet: all in the log')),
      !s.read_only && d.snapshot_interval > 0 && h('div', null,
        h('div', { class: 'meter', role: 'progressbar', 'aria-valuemin': 0, 'aria-valuemax': d.snapshot_interval, 'aria-valuenow': d.ops_since_snapshot, 'aria-label': 'Operations until the next automatic snapshot' },
          h('i', { style: { width: `${progress * 100}%` } })),
        h('div', { class: 'small faint' }, `${num(Math.max(0, d.snapshot_interval - d.ops_since_snapshot))} operations until the next automatic snapshot`)),
      s.read_only && h('p', { class: 'small faint', style: { 'margin-top': '8px' } }, "Whichever process owns this store writes its log and snapshots.")))
}

function healthPanel(s) {
  const checks = []
  if (s.zero_norm_vectors > 0) {
    checks.push(['warn', `${plural(s.zero_norm_vectors, 'vector')} ${s.zero_norm_vectors === 1 ? 'has' : 'have'} no direction (all zeros), so ${s.zero_norm_vectors === 1 ? 'it scores' : 'they score'} 0 against every query.`])
  } else {
    checks.push(['ok', 'Every vector has a direction.'])
  }
  const mixed = namespaces(s.groups).filter((r) => r.models.length > 1)
  if (mixed.length > 0) {
    const names = mixed.slice(0, 2).map((r) => `"${r.namespace}"`).join(' and ')
    checks.push(['warn', `${names}${mixed.length > 2 ? ` and ${mixed.length - 2} more` : ''} ${mixed.length === 1 ? 'mixes' : 'mix'} embedding models. Filter by model when searching there: scores across models mean nothing.`])
  } else if (s.groups.length > 0) {
    checks.push(['ok', 'Each namespace uses one embedding model.'])
  }
  if (s.read_only) {
    checks.push(s.durability.disk_changed
      ? ['warn', 'The store changed on disk since this copy loaded. Reload to catch up.']
      : ['ok', 'This copy matches the store on disk.'])
  }
  return h('section', { class: 'panel' },
    h('h2', null, 'Health'),
    h('div', { class: 'panel-body' },
      h('ul', { class: 'checks' }, checks.map(([kind, text]) => h('li', { class: kind }, icon(kind === 'ok' ? 'check' : 'warn'), h('span', null, text))))))
}

function empty(s, ctx) {
  if (s.read_only) {
    return h('section', { class: 'panel' }, emptyState({
      mark: true,
      title: 'This store is empty',
      body: 'Nothing has been written here yet. Add vectors from your application, or restart the dashboard as writable to import some.',
      actions: codeBlock(`athenadb --data ${shellQuote(s.path)} --writable`),
    }))
  }
  return h('section', { class: 'panel' }, emptyState({
    mark: true,
    title: 'This store is empty',
    body: 'Import your own vectors, or load a small synthetic sample to see how everything works.',
    actions: [
      button('Load sample data', { kind: 'primary', onClick: () => { requestSample(); ctx.navigate('import') } }),
      h('a', { class: 'btn', href: ctx.url('import'), style: { 'text-decoration': 'none' } }, 'Import vectors'),
    ],
  }))
}

function build(s, ctx) {
  if (s.vectors === 0) return [empty(s, ctx)]
  const models = new Set(s.groups.map((g) => g.embed_model_id).filter(Boolean))
  const untagged = s.groups.filter((g) => !g.embed_model_id).reduce((sum, g) => sum + g.vectors, 0)
  const names = new Set(s.groups.map((g) => g.namespace))
  const d = s.durability
  return [
    h('dl', { class: 'kpis', style: { margin: '0 0 12px' } },
      kpi('Vectors', num(s.vectors)),
      kpi('Dimensions', num(s.dimensions)),
      kpi('Namespaces', num(names.size)),
      kpi('Embedding models', num(models.size), untagged ? `${plural(untagged, 'vector')} untagged` : undefined),
      kpi('On disk', bytes(d.wal_bytes + d.snapshot_bytes), `log ${bytes(d.wal_bytes)} · snapshot ${bytes(d.snapshot_bytes)}`),
      kpi('Mean query', s.queries.count ? micros(s.queries.mean_micros) : '–',
        s.queries.count ? `over ${plural(s.queries.count, 'query', 'queries')}` : 'no queries yet')),
    h('div', { class: 'overview-grid' },
      namespacePanel(s, ctx),
      h('div', { class: 'stack' }, healthPanel(s), durabilityPanel(s, ctx))),
  ]
}

export function render(ctx) {
  const node = h('div', null, build(ctx.stats, ctx))
  return {
    node,
    update: (s) => {
      const active = document.activeElement
      const focused = node.contains(active) ? active.textContent : null
      fill(node, build(s, ctx))
      if (focused) [...node.querySelectorAll('a, button')].find((e) => e.textContent === focused)?.focus({ preventScroll: true })
    },
  }
}
