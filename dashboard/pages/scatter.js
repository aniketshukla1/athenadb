// A rotatable 3D point cloud on a 2D canvas. Drag to turn, scroll to zoom, click a point to
// select it; arrow keys and +/- do the same from the keyboard, and Esc clears the selection.
import { h } from '../ui.js'
import { seriesColors } from '../palette.js'

const CAMERA = 3.4
const SPIN_PER_MS = 0.00012
const PICK_RADIUS = 10
const DRAG_SLOP = 4

export class Scatter {
  constructor(stage, { onSelect, onHover, tipFor }) {
    this.onSelect = onSelect
    this.onHover = onHover
    this.tipFor = tipFor
    this.canvas = h('canvas', { tabindex: '0', role: 'img', 'aria-label': '3D view of the store. Arrow keys turn it, plus and minus zoom. The Search page has the same data as a list.' })
    this.tip = h('div', { class: 'tip', hidden: true })
    stage.append(this.canvas, this.tip)
    this.ctx = this.canvas.getContext('2d')
    this.points = []
    this.hidden = new Set()
    this.neighbours = new Map()
    this.selected = null
    this.hover = null
    this.rotX = -0.42
    this.rotY = 0.62
    this.zoom = 1
    this.spinning = !matchMedia('(prefers-reduced-motion: reduce)').matches
    this.colors = seriesColors()
    this.frame = 0
    this.last = 0
    this.listen()
    this.size()
    this.schedule()
  }

  listen() {
    const c = this.canvas
    let press = null
    this.handlers = [
      [c, 'pointerdown', (e) => {
        press = { x: e.clientX, y: e.clientY, rotX: this.rotX, rotY: this.rotY, moved: false }
        c.setPointerCapture(e.pointerId)
      }],
      [c, 'pointermove', (e) => {
        if (press) {
          const dx = e.clientX - press.x
          const dy = e.clientY - press.y
          if (Math.abs(dx) + Math.abs(dy) > DRAG_SLOP) press.moved = true
          if (press.moved) {
            this.rotY = press.rotY + dx * 0.008
            this.rotX = Math.max(-1.5, Math.min(1.5, press.rotX + dy * 0.008))
            this.schedule()
          }
          return
        }
        this.pick(e)
      }],
      [c, 'pointerup', (e) => {
        if (press && !press.moved) {
          this.pick(e)
          this.onSelect(this.hover)
        }
        press = null
      }],
      [c, 'pointercancel', () => { press = null }],
      [c, 'pointerleave', () => { if (!press) this.setHover(null) }],
      [c, 'wheel', (e) => {
        e.preventDefault()
        this.zoom = Math.max(0.5, Math.min(6, this.zoom * Math.exp(-e.deltaY * 0.0012)))
        this.schedule()
      }, { passive: false }],
      [c, 'keydown', (e) => {
        const step = 0.12
        const moves = {
          ArrowLeft: () => { this.rotY -= step },
          ArrowRight: () => { this.rotY += step },
          ArrowUp: () => { this.rotX = Math.max(-1.5, this.rotX - step) },
          ArrowDown: () => { this.rotX = Math.min(1.5, this.rotX + step) },
          '+': () => { this.zoom = Math.min(6, this.zoom * 1.15) },
          '=': () => { this.zoom = Math.min(6, this.zoom * 1.15) },
          '-': () => { this.zoom = Math.max(0.5, this.zoom / 1.15) },
          Escape: () => this.onSelect(null),
        }
        if (moves[e.key]) {
          e.preventDefault()
          moves[e.key]()
          this.schedule()
        }
      }],
      [window, 'athenadb-theme', () => this.recolor()],
    ]
    for (const [target, name, fn, options] of this.handlers) target.addEventListener(name, fn, options)
    this.scheme = matchMedia('(prefers-color-scheme: light)')
    this.onScheme = () => this.recolor()
    this.scheme.addEventListener('change', this.onScheme)
    this.resize = new ResizeObserver(() => this.size())
    this.resize.observe(this.canvas.parentElement)
  }

  recolor() {
    this.colors = seriesColors()
    this.schedule()
  }

  size() {
    const box = this.canvas.parentElement.getBoundingClientRect()
    if (!box.width || !box.height) return
    const dpr = Math.min(devicePixelRatio || 1, 2)
    this.canvas.width = Math.round(box.width * dpr)
    this.canvas.height = Math.round(box.height * dpr)
    this.w = box.width
    this.h = box.height
    this.dpr = dpr
    this.schedule()
  }

  /** points: [{ id, x, y, z, slot, namespace }]. */
  setPoints(points) {
    this.points = points
    this.pointScale = points.length < 100 ? 2 : points.length < 500 ? 1.4 : points.length > 5000 ? 0.75 : 1
    this.sx = new Float32Array(points.length)
    this.sy = new Float32Array(points.length)
    this.depth = new Float32Array(points.length)
    this.index = new Map(points.map((p, i) => [p.id, i]))
    this.selected = null
    this.neighbours = new Map()
    this.setHover(null)
    this.schedule()
  }

  setHidden(hidden) {
    this.hidden = hidden
    this.schedule()
  }

  setSpinning(on) {
    this.spinning = on
    this.last = 0
    this.schedule()
  }

  /** Mark a selected id and its neighbours, as ids that may or may not be in the sample. */
  setSelection(id, neighbourIds = []) {
    this.selected = id
    this.neighbours = new Map(neighbourIds.map((n, rank) => [n, rank]))
    if (id !== null && id === this.hover) this.tip.hidden = true
    this.schedule()
  }

  /** Light up a point from outside the canvas, as when hovering a row in the list. */
  highlight(id) {
    this.outsideHover = id
    this.schedule()
  }

  schedule() {
    if (!this.frame) this.frame = requestAnimationFrame((t) => this.tick(t))
  }

  tick(now) {
    this.frame = 0
    const moving = this.spinning && !this.hover && !this.selected
    if (moving) {
      if (this.last) this.rotY += (now - this.last) * SPIN_PER_MS * 6
      this.last = now
    } else {
      this.last = 0
    }
    this.draw()
    if (moving) this.schedule()
  }

  project(x, y, z) {
    const cy = Math.cos(this.rotY), sy = Math.sin(this.rotY)
    const cx = Math.cos(this.rotX), sx = Math.sin(this.rotX)
    const x1 = x * cy + z * sy
    const z1 = -x * sy + z * cy
    const y1 = y * cx - z1 * sx
    const z2 = y * sx + z1 * cx
    const persp = CAMERA / (CAMERA + z2)
    const radius = Math.min(this.w, this.h) * 0.42 * this.zoom
    return [this.w / 2 + x1 * radius * persp, this.h / 2 - y1 * radius * persp, z2, persp]
  }

  draw() {
    const { ctx, points, colors } = this
    if (!this.w) return
    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0)
    ctx.clearRect(0, 0, this.w, this.h)

    this.drawAxes()

    const n = points.length
    const order = new Array(n)
    for (let i = 0; i < n; i++) {
      const p = points[i]
      const [x, y, z] = this.project(p.x, p.y, p.z)
      this.sx[i] = x
      this.sy[i] = y
      this.depth[i] = z
      order[i] = i
    }
    order.sort((a, b) => this.depth[b] - this.depth[a])

    const selecting = this.selected !== null
    const selIndex = selecting ? this.index.get(this.selected) : undefined
    if (selIndex !== undefined) {
      ctx.strokeStyle = colors.lit
      ctx.lineWidth = 1
      ctx.globalAlpha = 0.65
      for (const id of this.neighbours.keys()) {
        const j = this.index.get(id)
        if (j === undefined) continue
        ctx.beginPath()
        ctx.moveTo(this.sx[selIndex], this.sy[selIndex])
        ctx.lineTo(this.sx[j], this.sy[j])
        ctx.stroke()
      }
    }

    for (const i of order) {
      const p = points[i]
      if (this.hidden.has(p.namespace)) continue
      if (p.id === this.selected || this.neighbours.has(p.id)) continue
      const near = 1 - (this.depth[i] + 1.8) / 3.6
      ctx.globalAlpha = (0.35 + 0.65 * near) * (selecting ? 0.28 : 1)
      ctx.fillStyle = p.slot >= 0 ? colors.slots[p.slot] : colors.other
      ctx.beginPath()
      ctx.arc(this.sx[i], this.sy[i], (1.4 + 1.7 * near) * this.pointScale, 0, Math.PI * 2)
      ctx.fill()
    }

    // Selection on top: a ring on a surface gap, so it reads over any colour.
    ctx.globalAlpha = 1
    for (const id of this.neighbours.keys()) {
      const j = this.index.get(id)
      if (j !== undefined && !this.hidden.has(points[j].namespace)) this.marker(j, 4, colors.lit)
    }
    if (selIndex !== undefined) this.marker(selIndex, 5.5, colors.fg)
    for (const id of [this.hover, this.outsideHover]) {
      if (id !== null && id !== undefined) this.ring(this.index.get(id))
    }
  }

  marker(i, radius, color) {
    const { ctx } = this
    ctx.fillStyle = color
    ctx.strokeStyle = this.colors.surface
    ctx.lineWidth = 2
    ctx.beginPath()
    ctx.arc(this.sx[i], this.sy[i], radius, 0, Math.PI * 2)
    ctx.stroke()
    ctx.fill()
  }

  ring(i) {
    if (i === undefined) return
    const { ctx } = this
    ctx.globalAlpha = 1
    ctx.strokeStyle = this.colors.fg
    ctx.lineWidth = 1.5
    ctx.beginPath()
    ctx.arc(this.sx[i], this.sy[i], 6, 0, Math.PI * 2)
    ctx.stroke()
  }

  drawAxes() {
    const { ctx, colors } = this
    ctx.lineWidth = 1
    ctx.strokeStyle = colors.line
    ctx.fillStyle = colors.faint
    ctx.font = '11px ui-monospace, SFMono-Regular, Menlo, monospace'
    ctx.globalAlpha = 0.8
    for (const [axis, label] of [[[1, 0, 0], 'PC1'], [[0, 1, 0], 'PC2'], [[0, 0, 1], 'PC3']]) {
      const [ax, ay] = this.project(-axis[0], -axis[1], -axis[2])
      const [bx, by] = this.project(axis[0], axis[1], axis[2])
      ctx.beginPath()
      ctx.moveTo(ax, ay)
      ctx.lineTo(bx, by)
      ctx.stroke()
      ctx.fillText(label, bx + 4, by + 4)
    }
    ctx.globalAlpha = 1
  }

  pick(e) {
    const box = this.canvas.getBoundingClientRect()
    const x = e.clientX - box.left
    const y = e.clientY - box.top
    let best = null
    let bestDistance = PICK_RADIUS * PICK_RADIUS
    for (let i = 0; i < this.points.length; i++) {
      if (this.hidden.has(this.points[i].namespace)) continue
      const dx = this.sx[i] - x
      const dy = this.sy[i] - y
      const d = dx * dx + dy * dy
      if (d < bestDistance) {
        bestDistance = d
        best = i
      }
    }
    this.setHover(best === null ? null : this.points[best].id, x, y)
  }

  setHover(id, x, y) {
    const changed = id !== this.hover
    this.hover = id
    this.canvas.style.setProperty('cursor', id === null ? 'grab' : 'pointer')
    if (id === null) {
      this.tip.hidden = true
    } else {
      const point = this.points[this.index.get(id)]
      this.tip.replaceChildren(...this.tipFor(point))
      this.tip.hidden = false
      this.tip.style.setProperty('left', `${Math.min(x + 12, this.w - 270)}px`)
      this.tip.style.setProperty('top', `${Math.min(y + 12, this.h - 54)}px`)
    }
    if (changed) {
      this.onHover?.(id)
      this.schedule()
    }
  }

  dispose() {
    cancelAnimationFrame(this.frame)
    for (const [target, name, fn] of this.handlers) target.removeEventListener(name, fn)
    this.scheme.removeEventListener('change', this.onScheme)
    this.resize.disconnect()
  }
}
