// The shell: routing, the status poll, the top bar.
import { api, setToken } from './api.js'
import { h, fill, icon, iconButton, button, field, errorState, errorNote, toast, emptyState } from './ui.js'
import * as overview from './pages/overview.js'
import * as explore from './pages/explore.js'
import * as search from './pages/search.js'
import * as browse from './pages/browse.js'
import * as importer from './pages/import.js'
import * as connect from './pages/connect.js'

const PAGES = [
  { key: 'overview', title: 'Overview', icon: 'overview', module: overview },
  { key: 'explore', title: 'Explore', icon: 'explore', module: explore },
  { key: 'search', title: 'Search', icon: 'search', module: search },
  { key: 'browse', title: 'Browse', icon: 'browse', module: browse },
  { key: 'import', title: 'Import', icon: 'import', module: importer },
  { key: 'connect', title: 'Connect', icon: 'connect', module: connect },
]
const POLL_MS = 5000

const main = document.getElementById('main')
const banner = document.getElementById('banner')
const topMeta = document.getElementById('top-meta')
const column = document.getElementById('column')

let stats = null
let current = null   // { key, instance }
let lastPrint = ''
let firstRoute = true
let triedToken = false

function parseRoute() {
  const hash = location.hash.startsWith('#/') ? location.hash.slice(2) : ''
  const [key, query] = hash.split('?')
  const page = PAGES.find((p) => p.key === key) || PAGES[0]
  return { page, params: Object.fromEntries(new URLSearchParams(query || '')) }
}

function routeUrl(key, params = {}) {
  const q = new URLSearchParams()
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== null && v !== '') q.set(k, v)
  }
  const s = q.toString()
  return `#/${key}${s ? `?${s}` : ''}`
}

const ctx = {
  get stats() { return stats },
  url: routeUrl,
  navigate: (key, params) => { location.hash = routeUrl(key, params) },
  /** Keep a page's filters in the URL without leaving the page. */
  setParams: (params) => { history.replaceState(null, '', routeUrl(current.key, params)) },
  refresh,
  rerender: route,
}

function renderNav(activeKey) {
  document.getElementById('nav').replaceChildren(...PAGES.map((p) => h('a', {
    class: 'nav-item',
    href: `#/${p.key}`,
    'aria-current': p.key === activeKey ? 'page' : undefined,
  }, icon(p.icon), p.title)))
}

function route() {
  if (!stats) return
  const { page, params } = parseRoute()
  current?.instance?.dispose?.()
  renderNav(page.key)
  document.getElementById('title').textContent = page.title
  document.title = `${page.title} · AthenaDB`
  const instance = page.module.render(ctx, params)
  current = { key: page.key, instance }
  main.replaceChildren(instance.node)
  column.scrollTop = 0
  // Tell keyboard and screen reader users the page changed, but not on first load.
  if (!firstRoute) main.focus({ preventScroll: true })
  firstRoute = false
  instance.mounted?.()
}

// ---- status -------------------------------------------------------------------------------

function printOf(s) {
  const { uptime_seconds, ...rest } = s
  return JSON.stringify(rest)
}

/** The last two folders: enough to recognise a store. The full path is in the tooltip. */
function shortPath(path) {
  const parts = path.split(/[/\\]/).filter(Boolean)
  return parts.length > 2 ? `…/${parts.slice(-2).join('/')}` : path
}

function renderTop() {
  const mode = stats.read_only
    ? h('a', { class: 'pill', href: '#/connect', title: 'Safe to inspect while another process writes. See Connect for how to enable changes.' }, icon('lock', 12), 'Read-only')
    : h('span', { class: 'pill write', title: 'This dashboard can change the store. It holds the writer lock, so nothing else can open the store for writing.' }, icon('pencil', 12), 'Writable')
  const changed = stats.durability.disk_changed
  fill(topMeta,
    h('span', { class: 'top-path truncate mono', title: stats.path }, shortPath(stats.path)),
    mode,
    stats.read_only && button(changed ? 'Reload: store changed' : 'Reload', {
      kind: changed ? 'primary' : 'ghost',
      iconName: 'reload',
      title: 'Load the store from disk again, to see what its writer has done',
      onClick: reload,
    }),
    themeToggle(),
  )
  document.getElementById('side-foot').replaceChildren(
    h('span', { class: 'num' }, `v${stats.version}`),
    h('a', { href: 'https://github.com/aniketshukla1/athenadb', target: '_blank', rel: 'noreferrer', class: 'faint' }, 'GitHub'),
  )
}

async function reload() {
  try {
    const { vectors } = await api.reload()
    await refresh()
    current?.instance?.reloaded?.()
    toast(`Reloaded: ${vectors.toLocaleString('en-US')} vectors`)
  } catch (e) {
    toast(e.message, 'error')
  }
}

const THEMES = ['auto', 'light', 'dark']

function currentTheme() {
  return document.documentElement.dataset.theme || 'auto'
}

function themeToggle() {
  const theme = currentTheme()
  const icons = { auto: 'auto', light: 'sun', dark: 'moon' }
  return iconButton(icons[theme], `Theme: ${theme}. Switch theme.`, {
    onClick: () => {
      const next = THEMES[(THEMES.indexOf(currentTheme()) + 1) % THEMES.length]
      if (next === 'auto') delete document.documentElement.dataset.theme
      else document.documentElement.dataset.theme = next
      try { localStorage.setItem('athenadb-theme', next) } catch { /* lasts until reload */ }
      renderTop()
      window.dispatchEvent(new Event('athenadb-theme'))
    },
  })
}

function showBanner(message) {
  banner.replaceChildren(...(message ? [h('div', { class: 'banner', role: 'alert' }, icon('warn'), message)] : []))
}

async function refresh() {
  try {
    const next = await api.stats()
    const firstLoad = !stats
    const print = printOf(next)
    const changed = print !== lastPrint
    stats = next
    lastPrint = print
    showBanner(null)
    if (firstLoad) {
      renderTop()
      route()
    } else if (changed) {
      renderTop()
      current?.instance?.update?.(stats)
    }
  } catch (e) {
    if (e.status === 401) return showTokenGate()
    if (stats) showBanner(`${e.message} Retrying.`)
    else main.replaceChildren(errorState(e, refresh))
  }
}

function showTokenGate() {
  const input = h('input', { type: 'password', autocomplete: 'current-password', required: true })
  renderNav('')
  document.getElementById('title').textContent = 'Sign in'
  main.replaceChildren(h('div', { class: 'gate panel' }, h('div', { class: 'panel-body' },
    h('form', {
      onSubmit: (e) => {
        e.preventDefault()
        setToken(input.value)
        triedToken = true
        stats = null
        main.replaceChildren(emptyState({ title: 'Checking the token…' }))
        refresh()
      },
    },
    h('p', { class: 'muted' }, 'This server was started with a token.'),
    triedToken && errorNote(new Error("That token didn't work.")),
    field('Token', input),
    h('button', { class: 'btn primary', type: 'submit' }, 'Continue')))))
  input.focus()
}

window.addEventListener('hashchange', () => { if (location.hash.startsWith('#/') || location.hash === '') route() })
document.getElementById('skip').addEventListener('click', () => main.focus())
document.addEventListener('visibilitychange', () => { if (!document.hidden) refresh() })
setInterval(() => { if (!document.hidden) refresh() }, POLL_MS)

refresh()
