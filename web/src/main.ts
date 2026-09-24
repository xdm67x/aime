const REDUCED = window.matchMedia('(prefers-reduced-motion: reduce)')

/* ------------------------------------------------------------------ */
/* Install command - every [data-copy] button copies the same command  */
/* ------------------------------------------------------------------ */

const INSTALL_CMD = document.querySelector<HTMLElement>('[data-install]')?.textContent?.trim() ?? ''

for (const btn of document.querySelectorAll<HTMLButtonElement>('[data-copy]')) {
    btn.addEventListener('click', async () => {
        const prev = btn.textContent
        try {
            await navigator.clipboard.writeText(INSTALL_CMD)
            btn.textContent = '✓ copied'
        } catch {
            btn.textContent = 'copy failed'
        }
        btn.classList.remove('pop')
        void btn.offsetWidth // restart the pop
        btn.classList.add('pop')
        setTimeout(() => {
            btn.textContent = prev
            btn.classList.remove('pop')
        }, 2000)
    })
}

/* ------------------------------------------------------------------ */
/* Scroll-linked motion: reveals glide with the scroll position and   */
/* the progress line runs across the top. JS-driven so it works in    */
/* every browser, not only where animation-timeline is enabled.       */
/* ------------------------------------------------------------------ */

const TRAVEL = 36 // px of rise over the first 40% of viewport entry

if (!REDUCED.matches) {
    document.documentElement.classList.add('js')
    const reveals = Array.from(document.querySelectorAll<HTMLElement>('.reveal'))
    const progress = document.querySelector<HTMLElement>('.progress')
    let ticking = false

    function frame() {
        ticking = false
        const vh = window.innerHeight
        const max = document.documentElement.scrollHeight - vh
        if (progress)
            progress.style.transform = `scaleX(${max > 0 ? Math.min(1, window.scrollY / max) : 0})`
        for (const el of reveals) {
            const top = el.getBoundingClientRect().top
            const p = Math.min(1, Math.max(0, (vh - top) / (vh * 0.4)))
            const e = 1 - Math.pow(1 - p, 3) // ease-out cubic: fast start, soft landing
            el.style.opacity = String(e)
            el.style.transform = `translateY(${((1 - e) * TRAVEL).toFixed(1)}px)`
        }
    }

    function request() {
        if (ticking) return
        ticking = true
        requestAnimationFrame(frame)
    }

    window.addEventListener('scroll', request, { passive: true })
    window.addEventListener('resize', request)
    frame()
}

/* ------------------------------------------------------------------ */
/* Hero: the file runs, in place                                       */
/* ------------------------------------------------------------------ */

const heroFile = document.getElementById('hero-file') as HTMLElement
const heroRuns = Array.from(heroFile.querySelectorAll<HTMLElement>('.ln.run'))
const heroState = document.getElementById('hero-state')
const heroCost = document.getElementById('hero-cost')

const HERO_COST = 0.012
const HERO_RUN_MS = 6900
const HERO_HOLD_MS = 4500
let heroToken = 0

function heroCount(token: number) {
    const t0 = performance.now()
    const step = () => {
        if (token !== heroToken) return
        const p = Math.min(1, (performance.now() - t0) / HERO_RUN_MS)
        const eased = 1 - Math.pow(1 - p, 3)
        const v = `$${(HERO_COST * eased).toFixed(4)}`
        if (heroCost) heroCost.textContent = v
        if (p < 1) {
            requestAnimationFrame(step)
        } else {
            if (heroState) heroState.textContent = 'workflow ship · goal reached'
        }
    }
    requestAnimationFrame(step)
}

function heroCycle() {
    const token = ++heroToken
    for (const el of heroRuns) {
        el.style.animation = 'none'
        void el.offsetWidth // restart the run-in animation
        el.style.animation = ''
    }
    if (heroState) heroState.textContent = 'workflow ship · running'
    if (heroCost) heroCost.textContent = '$0.0000'
    heroCount(token)
    setTimeout(heroCycle, HERO_RUN_MS + HERO_HOLD_MS)
}

if (REDUCED.matches) {
    if (heroState) heroState.textContent = 'workflow ship · goal reached'
    if (heroCost) heroCost.textContent = `$${HERO_COST.toFixed(4)}`
} else {
    heroCycle()
}
