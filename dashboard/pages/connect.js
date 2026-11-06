// How to reach this store from code, and what the read-only and writable modes mean.
import { h, tabs, codeBlock, icon, shellQuote } from '../ui.js'

const ENDPOINTS = [
  ['GET /api/stats', 'Counts, namespaces, disk use and query timing'],
  ['GET /api/vectors', 'List vectors. Filters: namespace, embed_model_id, q. Paging: limit, offset'],
  ['POST /api/vectors', 'Add or overwrite: {"vectors": [{"id", "vector", "metadata"?, "namespace"?, "embed_model_id"?}]}. One fsync per request'],
  ['GET /api/vectors/:id', 'One vector, with its values'],
  ['DELETE /api/vectors/:id', 'Delete. Safe to retry: {"deleted": true | false}'],
  ['POST /api/search', 'Top k by cosine: {"id"} or {"vector"}, with k, namespace, embed_model_id'],
  ['GET /api/projection', 'A 3D PCA of a sample of the store'],
  ['POST /api/snapshots', 'Write a snapshot now and empty the log'],
  ['POST /api/reload', 'Read-only servers: load the store from disk again'],
]

function snippets(stats) {
  const origin = location.origin
  const dim = stats.dimensions || 3
  const vector = `[${Array.from({ length: Math.min(dim, 3) }, (_, i) => [0.1, 0.2, 0.3][i]).join(', ')}${dim > 3 ? ', …' : ''}]`
  return {
    python: `import athenadb

db = athenadb.AthenaDB(${JSON.stringify(stats.path)}, read_only=True)
hits = db.query(${vector}, k=10)
for id, score, namespace, model, metadata in hits:
    print(id, round(score, 4))`,
    rust: `use athenadb::db::Database;

let db = Database::open_read_only(${JSON.stringify(stats.path)})?;
for hit in db.search(&query, 10, None, None)? {
    println!("{} {:.4}", hit.id, hit.score);
}`,
    http: `curl -s ${origin}/api/stats

curl -s ${origin}/api/search \\
  -H 'Content-Type: application/json' \\
  -d '{"vector": ${vector}, "k": 10}'`,
  }
}

export function render(ctx) {
  const stats = ctx.stats
  const code = snippets(stats)
  const box = h('div')
  const show = (key) => box.replaceChildren(codeBlock(code[key]))
  show('python')

  const modeNote = stats.read_only
    ? h('div', { class: 'stack' },
      h('p', null, h('strong', null, 'This server is read-only. '), 'It works from a copy of the store loaded at startup and never touches the files, so it\'s safe to run while your application writes. When the store changes on disk, the top bar offers a reload.'),
      h('p', { class: 'muted' }, 'To add, delete or snapshot from here, stop whatever else is writing to the store, then restart with:'),
      codeBlock(`athenadb --data ${shellQuote(stats.path)} --writable`))
    : h('div', { class: 'stack' },
      h('p', null, h('strong', null, 'This server is writable. '), 'It holds the store\'s writer lock: adds and deletes are logged and fsynced before they\'re acknowledged.'),
      h('div', { class: 'inline-note' }, icon('warn'), h('div', null, 'While it runs, anything else that opens this store for writing gets an error. To look at a store another process is writing to, run without --writable.')))

  return {
    node: h('div', { class: 'stack' },
      h('section', { class: 'panel' }, h('h2', null, 'Access'), h('div', { class: 'panel-body' }, modeNote)),
      h('section', { class: 'panel' },
        h('h2', null, 'From code'),
        h('div', { class: 'panel-body stack' },
          tabs([
            { value: 'python', label: 'Python' },
            { value: 'rust', label: 'Rust' },
            { value: 'http', label: 'HTTP' },
          ], 'python', show, 'Language'),
          box,
          h('p', { class: 'small faint' }, 'The Python and Rust examples don\'t go through this server. They open their own read-only copy, which works whatever else has the store open. Opening it for writing takes the writer lock, so that fails while another writer is running.'))),
      h('section', { class: 'panel' },
        h('h2', null, 'HTTP API'),
        h('div', { class: 'table-wrap', tabindex: '0' }, h('table', { class: 'api-table' },
          h('thead', null, h('tr', null, h('th', { scope: 'col' }, 'Endpoint'), h('th', { scope: 'col' }, 'What it does'))),
          h('tbody', null, ENDPOINTS.map(([route, what]) => h('tr', null, h('td', null, route), h('td', { class: 'muted' }, what))))))),
    ),
  }
}
