// Small DOM helpers and shared components. Ids and metadata are untrusted data, so everything
// here builds nodes and sets text. Nothing in the dashboard assigns innerHTML.

const SVG_NS = 'http://www.w3.org/2000/svg'

const ICONS = {
  overview: 'M3 3h8v8H3zM13 3h8v5h-8zM13 11h8v10h-8zM3 14h8v7H3z',
  explore: 'M12 10.2a1.8 1.8 0 1 0 0 3.6 1.8 1.8 0 0 0 0-3.6zM3 12c0-2.5 4-4.5 9-4.5s9 2 9 4.5-4 4.5-9 4.5-9-2-9-4.5zM7.5 19.8c-2.2-1.3-1.6-5.6 1-10s6-7 8.1-5.6 1.6 5.6-1 10-5.9 6.9-8.1 5.6z',
  search: 'M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.3-4.3',
  browse: 'M4 6h16M4 12h16M4 18h16',
  import: 'M12 3v12M7 10l5 5 5-5M4 21h16',
  connect: 'M9 2v6M15 2v6M6 8h12v4a6 6 0 0 1-12 0V8zM12 18v4',
  sun: 'M12 17a5 5 0 1 0 0-10 5 5 0 0 0 0 10zM12 1v2M12 21v2M4.2 4.2l1.4 1.4M18.4 18.4l1.4 1.4M1 12h2M21 12h2M4.2 19.8l1.4-1.4M18.4 5.6l1.4-1.4',
  moon: 'M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z',
  auto: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 3v18',
  reload: 'M21 12a9 9 0 1 1-2.6-6.4M21 3v6h-6',
  copy: 'M9 9h11v11H9zM5 15H4V4h11v1',
  check: 'M5 13l4 4L19 7',
  warn: 'M12 3l10 18H2L12 3zM12 10v5M12 18v.5',
  info: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 11v5M12 8v.5',
  close: 'M6 6l12 12M18 6L6 18',
  lock: 'M6 11h12v10H6zM8 11V7a4 4 0 0 1 8 0v4',
  pencil: 'M4 20l1-4L16 5l3 3L8 19l-4 1z',
  trash: 'M4 7h16M10 11v6M14 11v6M6 7l1 14h10l1-14M9 7V4h6v3',
  play: 'M8 5v14l11-7z',
  pause: 'M8 5v14M16 5v14',
  arrow: 'M5 12h14M13 6l6 6-6 6',
}

export function icon(name, size = 15) {
  const svg = document.createElementNS(SVG_NS, 'svg')
  svg.setAttribute('viewBox', '0 0 24 24')
  svg.setAttribute('width', size)
  svg.setAttribute('height', size)
  svg.setAttribute('fill', 'none')
  svg.setAttribute('stroke', 'currentColor')
  svg.setAttribute('stroke-width', '2')
  svg.setAttribute('stroke-linecap', 'round')
  svg.setAttribute('stroke-linejoin', 'round')
  svg.setAttribute('aria-hidden', 'true')
  const path = document.createElementNS(SVG_NS, 'path')
  path.setAttribute('d', ICONS[name])
  svg.append(path)
  return svg
}

const present = (children) => children.flat(Infinity).filter((c) => c !== null && c !== undefined && c !== false)

/** replaceChildren that skips empty children, as h() does. The native one prints them as text. */
export function fill(el, ...children) {
  el.replaceChildren(...present(children))
}

export function h(tag, props, ...children) {
  const el = document.createElement(tag)
  for (const [key, value] of Object.entries(props || {})) {
    if (value === undefined || value === null || value === false) continue
    if (key === 'class') el.className = value
    else if (key === 'style') {
      for (const [p, v] of Object.entries(value)) el.style.setProperty(p, v)
    } else if (key.startsWith('on')) el.addEventListener(key.slice(2).toLowerCase(), value)
    else if (key === 'value' || key === 'disabled' || key === 'checked' || key === 'selected' || key === 'hidden') el[key] = value
    else el.setAttribute(key, value === true ? '' : value)
  }
  el.append(...present(children))
  return el
}

// ---- formatting ---------------------------------------------------------------------------

export const num = (n) => Number(n).toLocaleString('en-US')

export function bytes(n) {
  if (n < 1024) return `${n} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let v = n
  let u = -1
  do { v /= 1024; u++ } while (v >= 1024 && u < units.length - 1)
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[u]}`
}

export function micros(us) {
  if (us < 1000) return `${us} µs`
  const ms = us / 1000
  return `${ms < 10 ? ms.toFixed(2) : ms < 100 ? ms.toFixed(1) : Math.round(ms)} ms`
}

export function ago(unixSeconds) {
  const s = Math.max(0, Math.round(Date.now() / 1000 - unixSeconds))
  if (s < 60) return 'just now'
  if (s < 3600) return `${Math.floor(s / 60)} min ago`
  if (s < 86400) return `${Math.floor(s / 3600)} hr ago`
  return `${Math.floor(s / 86400)} d ago`
}

export const fullDate = (unixSeconds) => new Date(unixSeconds * 1000).toLocaleString()

export const plural = (n, one, many = `${one}s`) => `${num(n)} ${n === 1 ? one : many}`

export function metadataPreview(metadata) {
  if (metadata === null || metadata === undefined) return ''
  return typeof metadata === 'string' ? metadata : JSON.stringify(metadata)
}

// ---- controls -----------------------------------------------------------------------------

export function button(label, props = {}) {
  const { kind = '', iconName, ...rest } = props
  return h('button', { type: 'button', class: `btn ${kind}`.trim(), ...rest }, iconName && icon(iconName), label)
}

export function iconButton(name, label, props = {}) {
  const { kind = 'ghost', ...rest } = props
  return h('button', { type: 'button', class: `btn icon ${kind}`, 'aria-label': label, title: label, ...rest }, icon(name))
}

export function field(label, control) {
  return h('label', { class: 'field' }, h('span', null, label), control)
}

/** A select over `options` (strings or {value, label}), with an "All" row when `all` is set. */
export function select(options, value, onChange, { all, id, label } = {}) {
  const rows = options.map((o) => (typeof o === 'object' ? o : { value: o, label: o || '(none)' }))
  return h('select', { id, 'aria-label': label, onChange: (e) => onChange(e.target.value) },
    all && h('option', { value: '' }, all),
    rows.map((o) => h('option', { value: o.value, selected: o.value === value }, o.label)))
}

/** Tabs: arrow keys move between them, and only the selected one is in the tab order. */
export function tabs(items, value, onChange, label) {
  const buttons = items.map((item) => h('button', {
    type: 'button',
    role: 'tab',
    'aria-selected': String(item.value === value),
    tabindex: item.value === value ? '0' : '-1',
    onClick: () => choose(item.value),
    onKeydown: (e) => {
      if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return
      const at = items.findIndex((i) => i.value === item.value)
      const next = items[(at + (e.key === 'ArrowRight' ? 1 : items.length - 1)) % items.length]
      choose(next.value)
      buttons[items.indexOf(next)].focus()
    },
  }, item.label))
  function choose(v) {
    buttons.forEach((b, i) => {
      const on = items[i].value === v
      b.setAttribute('aria-selected', String(on))
      b.tabIndex = on ? 0 : -1
    })
    onChange(v)
  }
  return h('div', { class: 'tabs', role: 'tablist', 'aria-label': label }, buttons)
}

export function copyButton(getText, label = 'Copy') {
  const b = iconButton('copy', label, {
    onClick: async () => {
      try {
        await navigator.clipboard.writeText(getText())
        toast('Copied')
      } catch {
        toast("Couldn't copy: the browser blocked it.", 'error')
      }
    },
  })
  return b
}

export function codeBlock(text) {
  return h('div', { class: 'code' }, h('pre', { tabindex: '0' }, h('code', null, text)), copyButton(() => text, 'Copy code'))
}

// ---- states -------------------------------------------------------------------------------

export function skeleton(rows = 6) {
  return h('div', { 'aria-busy': 'true', 'aria-label': 'Loading' },
    Array.from({ length: rows }, (_, i) => h('div', { class: 'skeleton', style: { width: `${88 - (i % 3) * 14}%` } })))
}

export function emptyState({ title, body, actions, mark }) {
  return h('div', { class: 'state' },
    mark && h('img', { src: '/assets/mark.svg', alt: '', width: 40, height: 40 }),
    h('h2', null, title),
    body && h('p', null, body),
    actions && h('div', { class: 'actions' }, actions))
}

export function errorNote(error) {
  return h('div', { class: 'inline-error', role: 'alert' }, icon('warn'), h('div', null, error.message || String(error)))
}

export function errorState(error, retry) {
  return h('div', { class: 'state' },
    h('h2', null, "That didn't load"),
    h('p', null, error.message || String(error)),
    retry && h('div', { class: 'actions' }, button('Try again', { onClick: retry })))
}

/** Says why something can't be done here, with the exact command that enables it. */
export function readOnlyNote(stats, what) {
  const command = `athenadb --data ${shellQuote(stats.path)} --writable`
  return h('div', { class: 'stack' },
    h('div', { class: 'inline-note' }, icon('lock'),
      h('div', null, `This store is open read-only, so ${what}. That keeps it safe to inspect while another process writes to it. To make changes, stop any other process using the store and restart the dashboard with:`)),
    codeBlock(command))
}

export function shellQuote(s) {
  return /^[\w@%+=:,./-]+$/.test(s) ? s : `'${String(s).replace(/'/g, "'\\''")}'`
}

// ---- overlays -----------------------------------------------------------------------------

export function toast(message, kind = 'ok') {
  const el = h('div', { class: `toast ${kind}` }, icon(kind === 'error' ? 'warn' : 'check'), h('div', null, message))
  document.getElementById('toasts').append(el)
  setTimeout(() => el.remove(), kind === 'error' ? 8000 : 3500)
}

function modal(dialog) {
  document.body.append(dialog)
  dialog.addEventListener('close', () => dialog.remove())
  // A click on the backdrop lands on the dialog element itself.
  dialog.addEventListener('click', (e) => { if (e.target === dialog) dialog.close() })
  dialog.showModal()
  return dialog
}

/** A native modal: it traps focus, Esc closes it, and it returns focus to whatever opened it. */
export function openDrawer(title, body, foot, headAction) {
  const dialog = h('dialog', { class: 'drawer', 'aria-label': title },
    h('div', { class: 'drawer-head' },
      h('h2', { class: 'grow truncate', title }, title),
      headAction,
      iconButton('close', 'Close', { onClick: () => dialog.close() })),
    h('div', { class: 'drawer-body' }, body),
    foot && h('div', { class: 'drawer-foot' }, foot))
  return modal(dialog)
}

export function confirmDialog({ title, body, confirm, danger }) {
  return new Promise((resolve) => {
    let answer = false
    const dialog = h('dialog', { class: 'confirm', 'aria-label': title },
      h('h2', null, title),
      h('p', null, body),
      h('div', { class: 'actions' },
        button('Cancel', { autofocus: true, onClick: () => dialog.close() }),
        button(confirm, { kind: danger ? 'danger solid' : 'primary', onClick: () => { answer = true; dialog.close() } })))
    dialog.addEventListener('close', () => resolve(answer))
    modal(dialog)
  })
}

/** A vector at a glance: each value as a cell, coloured by sign and size. */
export function fingerprint(values) {
  const cells = Math.min(values.length, 128)
  const canvas = h('canvas', { class: 'fingerprint', role: 'img', 'aria-label': `Heat strip of ${values.length} values` })
  canvas.width = cells
  canvas.height = 1
  const css = getComputedStyle(document.documentElement)
  const pos = css.getPropertyValue('--lit').trim()
  const neg = css.getPropertyValue('--accent').trim()
  const ctx = canvas.getContext('2d')
  const max = values.reduce((m, v) => Math.max(m, Math.abs(v)), 0) || 1
  for (let i = 0; i < cells; i++) {
    const v = values[Math.floor((i * values.length) / cells)]
    ctx.globalAlpha = 0.12 + 0.88 * (Math.abs(v) / max)
    ctx.fillStyle = v >= 0 ? pos : neg
    ctx.fillRect(i, 0, 1, 1)
  }
  canvas.style.setProperty('image-rendering', 'pixelated')
  return canvas
}
