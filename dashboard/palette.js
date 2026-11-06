// Series colours for namespaces, validated for contrast and colour blindness in both themes
// (see --s1 to --s6 in style.css). A namespace keeps its colour however the data is filtered,
// and anything past the sixth folds into "other" rather than getting a generated hue.

const SLOTS = 6

export function seriesColors() {
  const css = getComputedStyle(document.documentElement)
  const read = (name) => css.getPropertyValue(name).trim()
  return {
    slots: Array.from({ length: SLOTS }, (_, i) => read(`--s${i + 1}`)),
    other: read('--s-other'),
    lit: read('--lit'),
    fg: read('--fg'),
    faint: read('--faint'),
    line: read('--line-strong'),
    surface: read('--surface'),
  }
}

function hash(name) {
  let n = 0
  for (const c of name) n = (Math.imul(n, 31) + c.codePointAt(0)) | 0
  return Math.abs(n)
}

/**
 * Assign from the whole store, never the current filter: the six largest namespaces each take the
 * slot their name hashes to, or the next free one, so adding a namespace leaves the others' colours
 * alone. Returns -1 for "other".
 */
export function assignColors(groups) {
  const counts = new Map()
  for (const g of groups) counts.set(g.namespace, (counts.get(g.namespace) || 0) + g.vectors)
  const top = [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, SLOTS)
  const slotOf = new Map()
  const taken = new Set()
  for (const [name] of top) {
    let slot = hash(name) % SLOTS
    while (taken.has(slot)) slot = (slot + 1) % SLOTS
    taken.add(slot)
    slotOf.set(name, slot)
  }
  return (namespace) => slotOf.get(namespace) ?? -1
}
