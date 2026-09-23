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
            btn.textContent = 'Copied!'
        } catch {
            btn.textContent = 'Copy failed'
        }
        setTimeout(() => {
            btn.textContent = prev
        }, 2000)
    })
}

/* ------------------------------------------------------------------ */
/* Scroll reveals - sections settle in as they enter the viewport      */
/* ------------------------------------------------------------------ */

const reveals = Array.from(document.querySelectorAll<HTMLElement>('.reveal'))

if (!REDUCED.matches && 'IntersectionObserver' in window) {
    document.documentElement.classList.add('js')
    const io = new IntersectionObserver(
        (entries) => {
            for (const entry of entries) {
                if (!entry.isIntersecting) continue
                entry.target.classList.add('in')
                io.unobserve(entry.target)
            }
        },
        { threshold: 0.15 },
    )
    for (const el of reveals) io.observe(el)
} else {
    for (const el of reveals) el.classList.add('in')
}

/* ------------------------------------------------------------------ */
/* Slide deck - vertical scroll pans the slides left to right           */
/* ------------------------------------------------------------------ */

const HORIZONTAL_MIN = 941

const deck = document.getElementById('deck') as HTMLElement
const deckViewport = deck.querySelector<HTMLElement>('.deck-viewport') as HTMLElement
const deckTrack = deck.querySelector<HTMLElement>('.deck-track') as HTMLElement
const slides = Array.from(deckTrack.querySelectorAll<HTMLElement>('.slide'))
const ddots = Array.from(document.querySelectorAll<HTMLButtonElement>('.deck-nav .ddot'))
const countCur = document.getElementById('count-cur')

let horizontal = false
let vw = 0
let vh = 0
let maxX = 0
let currentX = 0
let activeIdx = -1
const LOCK_MS = 800
let lockUntil = 0
let wheelAcc = 0

/** Viewport width excluding the scrollbar, matching the CSS media queries. */
function viewportWidth() {
    return document.documentElement.clientWidth
}

function layout() {
    vw = viewportWidth()
    vh = window.innerHeight
    deck.style.height = `${slides.length * vh}px`
    deckViewport.style.height = `${vh}px`
    for (const slide of slides) slide.style.width = `${vw}px`
    maxX = (slides.length - 1) * vw
}

function replayAnimations(slide: HTMLElement) {
    for (const el of slide.querySelectorAll<HTMLElement>('.rise, .reveal')) {
        el.style.animation = 'none'
        void el.offsetWidth // restart the entrance animation
        el.style.animation = ''
    }
}

function setActive(idx: number) {
    if (idx === activeIdx) return
    activeIdx = idx
    slides.forEach((slide, i) => slide.classList.toggle('active', i === idx))
    ddots.forEach((dot, i) => {
        dot.classList.toggle('active', i === idx)
        if (i === idx) dot.setAttribute('aria-current', 'true')
        else dot.removeAttribute('aria-current')
    })
    if (countCur) countCur.textContent = String(idx + 1).padStart(2, '0')
    if (horizontal) replayAnimations(slides[idx] as HTMLElement)
}

function setMode(next: boolean) {
    if (next === horizontal) return
    horizontal = next
    document.documentElement.classList.toggle('js-deck', next)
    if (next) {
        layout()
        currentX = 0
        setActive(0)
    } else {
        deck.style.height = ''
        deckViewport.style.height = ''
        deckTrack.style.transform = ''
        for (const slide of slides) slide.style.width = ''
        for (const slide of slides) slide.classList.remove('active')
        activeIdx = -1
        currentX = 0
    }
}

function frame() {
    if (horizontal) {
        const top = deck.getBoundingClientRect().top
        const total = Math.max(1, slides.length * vh - vh)
        const p = Math.min(1, Math.max(0, -top / total))
        const targetX = p * maxX
        if (Math.abs(targetX - currentX) > 0.1) {
            currentX += (targetX - currentX) * 0.14
            if (Math.abs(targetX - currentX) < 0.5) currentX = targetX
            deckTrack.style.transform = `translate3d(${-currentX}px, 0, 0)`
        }
        /* derive the active slide from the smoothed position, so the dot
           flips exactly once per transition instead of jittering */
        setActive(Math.round(currentX / Math.max(1, vw)))
    }
    requestAnimationFrame(frame)
}

function goToSlide(i: number) {
    const clamped = Math.min(slides.length - 1, Math.max(0, i))
    const top = deck.getBoundingClientRect().top + window.scrollY
    const total = slides.length * vh - vh
    const target = top + (clamped / (slides.length - 1)) * total
    if (Math.abs(target - window.scrollY) < 2) return
    lockUntil = performance.now() + LOCK_MS
    window.scrollTo({ top: target, behavior: 'smooth' })
}

for (const dot of ddots) {
    dot.addEventListener('click', () => {
        goToSlide(Number(dot.dataset.slide ?? 0))
    })
}

/* each wheel gesture advances exactly one slide; a lock during the
   animation keeps trackpad momentum from chaining extra jumps */
window.addEventListener(
    'wheel',
    (e) => {
        if (!horizontal || e.ctrlKey) return
        e.preventDefault()
        const now = performance.now()
        if (now < lockUntil) {
            // extend the lock while momentum keeps firing events
            lockUntil = Math.min(now + 250, lockUntil + 100)
            wheelAcc = 0
            return
        }
        const delta = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY
        wheelAcc += delta
        if (Math.abs(wheelAcc) < 40) return
        const next = activeIdx + (wheelAcc > 0 ? 1 : -1)
        wheelAcc = 0
        goToSlide(next)
    },
    { passive: false },
)

window.addEventListener('keydown', (e) => {
    if (!horizontal || e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey) return
    if (performance.now() < lockUntil) return
    if (e.key === 'ArrowRight' || e.key === 'PageDown') {
        e.preventDefault()
        goToSlide(activeIdx + 1)
    } else if (e.key === 'ArrowLeft' || e.key === 'PageUp') {
        e.preventDefault()
        goToSlide(activeIdx - 1)
    }
})

window.addEventListener('resize', () => {
    const next = !REDUCED.matches && viewportWidth() >= HORIZONTAL_MIN
    if (next !== horizontal) setMode(next)
    else if (horizontal) layout()
})

setMode(!REDUCED.matches && viewportWidth() >= HORIZONTAL_MIN)
requestAnimationFrame(frame)

/* ------------------------------------------------------------------ */
/* Hero run loop - the terminal replays a beat forever, the nav ticker  */
/* mirrors it: running, counting cost, done, next beat                  */
/* ------------------------------------------------------------------ */

const heroTerm = document.querySelector<HTMLElement>('.hero .term') as HTMLElement
const heroAnimEls = Array.from(heroTerm.querySelectorAll<HTMLElement>('.t-line, .t-type'))
const heroBeatEl = heroTerm.querySelector<HTMLElement>('.ts-left') as HTMLElement
const heroCostEl = heroTerm.querySelector<HTMLElement>('.ts-cost') as HTMLElement
const navTicker = document.getElementById('nav-ticker')
const tickDot = document.getElementById('tick-dot')
const tickBeat = document.getElementById('tick-beat')
const tickState = document.getElementById('tick-state')
const tickCost = document.getElementById('tick-cost')

const HERO_COST = 0.312
const HERO_RUN_MS = 5700
const HERO_HOLD_MS = 4200
let heroBeat = 12
let heroToken = 0

function heroCountCost(token: number) {
    const t0 = performance.now()
    const step = () => {
        if (token !== heroToken) return
        const p = Math.min(1, (performance.now() - t0) / HERO_RUN_MS)
        const eased = 1 - Math.pow(1 - p, 3)
        const v = `$${(HERO_COST * eased).toFixed(4)}`
        heroCostEl.textContent = v
        if (tickCost) tickCost.textContent = v
        if (p < 1) {
            requestAnimationFrame(step)
        } else {
            heroCostEl.textContent = `$${HERO_COST.toFixed(4)}`
            if (tickCost) tickCost.textContent = `$${HERO_COST.toFixed(4)}`
            if (tickState) tickState.textContent = 'done'
            if (tickDot) tickDot.classList.remove('on')
            navTicker?.classList.remove('running')
        }
    }
    requestAnimationFrame(step)
}

function heroCycle() {
    const token = ++heroToken
    for (const el of heroAnimEls) {
        el.style.animation = 'none'
        void el.offsetWidth // restart the CSS animation
        el.style.animation = ''
    }
    heroBeat += 1
    heroBeatEl.textContent = `beat-${heroBeat} · workflow base`
    heroCostEl.textContent = '$0.0000'
    if (tickBeat) tickBeat.textContent = `beat-${heroBeat}`
    if (tickState) tickState.textContent = 'running'
    if (tickDot) tickDot.classList.add('on')
    navTicker?.classList.add('running')
    heroCountCost(token)
    setTimeout(heroCycle, HERO_RUN_MS + HERO_HOLD_MS)
}

if (REDUCED.matches) {
    heroBeatEl.textContent = 'beat-12 · workflow base'
    heroCostEl.textContent = `$${HERO_COST.toFixed(4)}`
    if (tickBeat) tickBeat.textContent = 'beat-12'
    if (tickState) tickState.textContent = 'done'
    if (tickDot) tickDot.classList.remove('on')
    navTicker?.classList.remove('running')
} else {
    heroCycle()
}

/* ------------------------------------------------------------------ */
/* Workflow playground                                                 */
/* ------------------------------------------------------------------ */

interface WfStep {
    name: string
    model: string
    prompt: string
    tools: string[]
    tokens: number
    cost: number
}

interface Workflow {
    blurb: string
    userPrompt: string
    steps: WfStep[]
}

const WORKFLOWS: Record<string, Workflow> = {
    base: {
        blurb: 'The default. Any plain prompt runs straight through one step: one model, the full agentic loop, one beat.',
        userPrompt: 'add a rate limiter to the api layer',
        steps: [
            {
                name: 'run',
                model: 'anthropic/claude-3.5-sonnet',
                prompt: '"{{prompt}}"',
                tools: [
                    '⋯ grep "limiter" src/api - 9 matches',
                    '⋯ read_file api/router.go - 132 lines',
                    '⋯ edit_file limiter.go - token bucket added',
                    '✓ bash go test ./... - 18 passed',
                ],
                tokens: 1200,
                cost: 0.0142,
            },
        ],
    },
    review: {
        blurb: 'Two steps, two models. Claude reads and plans, Mistral applies the patch: one prompt, one beat.',
        userPrompt: 'review the auth module',
        steps: [
            {
                name: 'plan',
                model: 'anthropic/claude-3.5-sonnet',
                prompt: '"Review {{prompt}} and outline fixes"',
                tools: ['⋯ read_file auth.rs - 214 lines', '⋯ grep "session" src - 5 matches'],
                tokens: 800,
                cost: 0.0091,
            },
            {
                name: 'fix',
                model: 'mistral-large-latest',
                prompt: '"Apply the plan above"',
                tools: ['⋯ edit_file session expiry check added', '✓ bash cargo test - 41 passed'],
                tokens: 1500,
                cost: 0.0068,
            },
        ],
    },
    ship: {
        blurb: 'The release routine. The first step proves the build is green, the second one tags and ships it.',
        userPrompt: 'release the api changes',
        steps: [
            {
                name: 'test',
                model: 'anthropic/claude-3.5-sonnet',
                prompt: '"Run the full test suite for {{prompt}}"',
                tools: ['⋯ bash cargo test - 41 passed, 0 failed'],
                tokens: 600,
                cost: 0.0071,
            },
            {
                name: 'release',
                model: 'mistral-large-latest',
                prompt: '"Bump the version, tag and build"',
                tools: ['⋯ edit_file Cargo.toml - 0.10.1', '✓ bash git tag v0.10.1'],
                tokens: 1100,
                cost: 0.0049,
            },
        ],
    },
}

const pgYaml = document.getElementById('pg-yaml') as HTMLElement
const pgTrace = document.getElementById('pg-trace') as HTMLElement
const pgTitle = document.getElementById('pg-title') as HTMLElement
const pgFile = document.getElementById('pg-file') as HTMLElement
const pgBlurb = document.getElementById('pg-blurb') as HTMLElement
const pgStatus = document.getElementById('pg-status') as HTMLElement
const pgCost = document.getElementById('pg-cost') as HTMLElement
const pgStatusBar = document.querySelector<HTMLElement>('.pg-status')
const pgBeats = document.getElementById('pg-beats') as HTMLElement
const pgRun = document.getElementById('pg-run') as HTMLButtonElement
const chips = Array.from(document.querySelectorAll<HTMLButtonElement>('.chip'))

let runToken = 0
let beatNo = 11
let currentWf = 'base'

function sleep(ms: number) {
    return new Promise<void>((resolve) => setTimeout(resolve, ms))
}

/** Wait, then report whether this run is still the live one. */
async function wait(token: number, ms: number) {
    await sleep(REDUCED.matches ? 0 : ms)
    return token === runToken
}

type Tok = { cls: string; text: string }

function key(text: string): Tok {
    return { cls: 'y-key', text }
}
function str(text: string): Tok {
    return { cls: 'y-str', text }
}
function dash(text: string): Tok {
    return { cls: 'y-dash', text }
}
function mut(text: string): Tok {
    return { cls: 'y-mut', text }
}

function yamlLines(wf: Workflow) {
    const lines: Tok[][] = []
    const push = (...toks: Tok[]) => lines.push(toks)
    push(key('name:'), str(currentWf))
    push(key('steps:'))
    for (const step of wf.steps) {
        push(dash('-'), key('name:'), str(step.name))
        push(mut('   '), key('model:'), str(step.model))
        push(mut('   '), key('prompt:'), str(step.prompt))
    }
    return lines
}

function renderYaml(wf: Workflow) {
    pgYaml.replaceChildren()
    const lines = yamlLines(wf)
    lines.forEach((line, i) => {
        const row = document.createElement('div')
        row.className = 'y-line'
        row.style.setProperty('--d', `${(i * 0.09).toFixed(2)}s`)
        for (const tok of line) {
            const span = document.createElement('span')
            span.className = tok.cls
            span.textContent = tok.text
            row.append(span)
        }
        pgYaml.append(row)
    })
    return lines.length * 90 + 300
}

function addLine(cls: string, html: string) {
    const line = document.createElement('div')
    line.className = `t-line ${cls}`
    line.innerHTML = html
    pgTrace.append(line)
}

function addTyped(text: string) {
    const line = document.createElement('div')
    line.className = 't-line user'
    const prompt = document.createElement('span')
    prompt.className = 't-prompt'
    prompt.textContent = '▸'
    const type = document.createElement('span')
    type.className = 't-type'
    type.textContent = text
    type.style.setProperty('--w', `${text.length}ch`)
    type.style.setProperty('--ts', String(text.length))
    type.style.setProperty('--tw', `${Math.max(0.4, text.length * 0.045).toFixed(2)}s`)
    type.style.setProperty('--td', '0.1s')
    line.append(prompt, type)
    pgTrace.append(line)
    return text.length * 45 + 350
}

function startTicker(token: number, total: number, durationMs: number) {
    if (REDUCED.matches) {
        pgCost.textContent = `$${total.toFixed(4)}`
        return
    }
    const t0 = performance.now()
    const step = () => {
        if (token !== runToken) return
        const p = Math.min(1, (performance.now() - t0) / durationMs)
        const eased = 1 - Math.pow(1 - p, 3)
        pgCost.textContent = `$${(total * eased).toFixed(4)}`
        if (p < 1) requestAnimationFrame(step)
        else pgCost.textContent = `$${total.toFixed(4)}`
    }
    requestAnimationFrame(step)
}

async function runWorkflow(id: string) {
    const token = ++runToken
    currentWf = id
    const wf = WORKFLOWS[id]
    if (!wf) return

    chips.forEach((chip) => {
        const active = chip.dataset.wf === id
        chip.classList.toggle('active', active)
        chip.setAttribute('aria-selected', String(active))
    })

    pgBlurb.textContent = wf.blurb
    pgTitle.textContent = id === 'base' ? 'pulse' : `pulse - /workflow ${id}`
    pgFile.textContent = `~/.pulse/workflows/${id}.yml`
    pgTrace.replaceChildren()
    pgCost.textContent = '$0.0000'
    beatNo += 1

    /* log the beat: a running row that settles to done with its cost */
    pgBeats.querySelectorAll('li.running').forEach((row) => row.remove())
    const beatRow = document.createElement('li')
    beatRow.className = 'running'
    const rowDot = document.createElement('span')
    rowDot.className = 'status-dot on'
    const rowId = document.createElement('span')
    rowId.textContent = `beat-${beatNo}`
    const rowWf = document.createElement('span')
    rowWf.className = 'pg-beat-wf'
    rowWf.textContent = id
    const rowCost = document.createElement('span')
    rowCost.className = 'pg-beat-cost'
    rowCost.textContent = '$0.0000'
    beatRow.append(rowDot, rowId, rowWf, rowCost)
    pgBeats.append(beatRow)
    while (pgBeats.children.length > 5) pgBeats.firstElementChild?.remove()

    const yamlMs = renderYaml(wf)
    pgStatus.textContent = `beat-${beatNo} - workflow ${id} - loading yaml`
    if (!(await wait(token, yamlMs))) return

    const totalCost = wf.steps.reduce((sum, s) => sum + s.cost, 0)
    const totalTokens = wf.steps.reduce((sum, s) => sum + s.tokens, 0)
    const runMs = 900 + wf.steps.reduce((sum, s) => sum + 500 + s.tools.length * 520 + 400, 0)
    startTicker(token, totalCost, runMs)
    pgStatusBar?.classList.add('running')
    pgStatus.textContent = `beat-${beatNo} - workflow ${id} - running`

    const typeMs = addTyped(wf.userPrompt)
    if (!(await wait(token, typeMs))) return

    let tokensSoFar = 0
    for (const [i, step] of wf.steps.entries()) {
        addLine('ok', `→ step ${i + 1}/${wf.steps.length} - ${step.name} - ${step.model}`)
        if (!(await wait(token, 550))) return
        for (const tool of step.tools) {
            addLine(tool.startsWith('✓') ? 'ok' : 'tool', tool)
            if (!(await wait(token, 520))) return
        }
        tokensSoFar += step.tokens
        addLine('ai', `step ${step.name} done - ${(tokensSoFar / 1000).toFixed(1)}k tokens`)
        if (!(await wait(token, 450))) return
    }

    addLine(
        'ai',
        `beat saved - ${(totalTokens / 1000).toFixed(1)}k tokens - <span class="cost">$${totalCost.toFixed(4)}</span>`,
    )
    pgCost.textContent = `$${totalCost.toFixed(4)}`
    pgStatus.textContent = `beat-${beatNo} - workflow ${id} - done`
    pgStatusBar?.classList.remove('running')

    /* the logged beat settles: dot goes quiet, cost lands */
    beatRow.classList.remove('running')
    rowDot.classList.remove('on')
    rowCost.textContent = `$${totalCost.toFixed(4)}`
}

for (const chip of chips) {
    chip.addEventListener('click', () => {
        void runWorkflow(chip.dataset.wf ?? 'base')
    })
    chip.addEventListener('keydown', (e) => {
        if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return
        e.preventDefault()
        const dir = e.key === 'ArrowDown' ? 1 : -1
        const next = (chips.indexOf(chip) + dir + chips.length) % chips.length
        chips[next]?.focus()
        chips[next]?.click()
    })
}

pgRun.addEventListener('click', () => {
    pgRun.classList.remove('spinning')
    void pgRun.offsetWidth // restart the spin animation
    pgRun.classList.add('spinning')
    void runWorkflow(currentWf)
})

/* First run kicks off when the playground section comes into view */
const playground = document.getElementById('playground') as HTMLElement
if (REDUCED.matches || !('IntersectionObserver' in window)) {
    void runWorkflow('base')
} else {
    let started = false
    const io = new IntersectionObserver(
        (entries) => {
            if (started || !entries.some((e) => e.isIntersecting)) return
            started = true
            io.disconnect()
            void runWorkflow('base')
        },
        { threshold: 0.35 },
    )
    io.observe(playground)
}
