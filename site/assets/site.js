// The page is complete without any of this: the logo is an addition, and the code stays readable.

// Copy buttons
document.querySelectorAll('[data-copy]').forEach((box) => {
  const button = box.querySelector('.copy')
  button.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(box.querySelector('code').innerText)
      button.textContent = 'Copied'
    } catch {
      button.textContent = 'Press ⌘C'
    }
    setTimeout(() => { button.textContent = 'Copy' }, 1600)
  })
})

// Language tabs, with arrow keys
const tabs = [...document.querySelectorAll('[role="tab"]')]
const select = (tab) => {
  tabs.forEach((t) => {
    const on = t === tab
    t.setAttribute('aria-selected', String(on))
    t.tabIndex = on ? 0 : -1
    document.getElementById(t.getAttribute('aria-controls')).hidden = !on
  })
}
tabs.forEach((tab, i) => {
  tab.addEventListener('click', () => select(tab))
  tab.addEventListener('keydown', (e) => {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return
    const next = tabs[(i + (e.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length]
    select(next)
    next.focus()
  })
})

// The live 3D logo. Same engine and timeline as the README's animation (see Glaux: scripts/logo3d).
const LOOP_MS = 12000
const canvas = document.getElementById('logo')
const art = canvas.parentElement
const probe = document.createElement('canvas')

if (!(probe.getContext('webgl2') || probe.getContext('webgl'))) {
  art.classList.add('no-webgl')
} else {
  import('./swarm-logo.min.js').then(({ createSwarmLogo }) => {
    const small = matchMedia('(max-width: 767px)').matches
    const logo = createSwarmLogo({
      canvas, mark: 'athenadb', scale: 1.42, galaxy: 1.75, background: 0x000000,
      density: small ? 0.5 : 1, bloom: !small, dust: small ? 160 : 420,
    })
    const fit = () => {
      const box = canvas.getBoundingClientRect()
      if (box.width) logo.resize(box.width, box.height, Math.min(devicePixelRatio || 1, 1.75))
    }
    fit()

    if (matchMedia('(prefers-reduced-motion: reduce)').matches) {
      const still = () => { fit(); logo.render(0.1) }
      still()
      addEventListener('resize', still)
      canvas.classList.add('live')
      return
    }

    // Start just before the swarm collapses, so the reveal is the first thing a visitor sees.
    const start = performance.now() - 0.6 * LOOP_MS
    let raf = 0, visible = true, px = 0, py = 0, x = 0, y = 0, last = 0, slow = 0, seen = 0
    const tick = (now) => {
      raf = 0
      if (!visible) return
      x += (px - x) * 0.06
      y += (py - y) * 0.06
      logo.render(((now - start) / LOOP_MS) % 1, { x, y })
      if (last && seen < 120) {   // a device that can't hold 30 fps loses the bloom, not the logo
        seen++
        if (now - last > 34) slow++
        if (seen === 120 && slow > 70) logo.setBloom(false)
      }
      last = now
      raf = requestAnimationFrame(tick)
    }
    addEventListener('pointermove', (e) => {
      px = (e.clientX / innerWidth - 0.5) * 0.5
      py = (e.clientY / innerHeight - 0.5) * 0.3
    }, { passive: true })
    new ResizeObserver(fit).observe(canvas)
    new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting
      if (visible && !raf) { last = 0; raf = requestAnimationFrame(tick) }
    }).observe(canvas)
    raf = requestAnimationFrame(tick)
    canvas.classList.add('live')
  }).catch(() => art.classList.add('no-webgl'))
}
