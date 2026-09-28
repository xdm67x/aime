/* ------------------------------------------------------------------ */
/* Pulse site: copy buttons, the demo run, section reveals.            */
/* ------------------------------------------------------------------ */

const REDUCED = window.matchMedia('(prefers-reduced-motion: reduce)')
const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms))

/* ------------------------------------------------------------------ */
/* Copy buttons                                                        */
/* ------------------------------------------------------------------ */

for (const btn of document.querySelectorAll<HTMLButtonElement>('[data-copy]')) {
    btn.addEventListener('click', async () => {
        const chip = btn.closest('.cmd-chip')
        const cmd = btn.dataset.cmd ?? chip?.querySelector('code')?.textContent?.trim() ?? ''
        try {
            await navigator.clipboard.writeText(cmd)
            btn.textContent = 'copied'
            btn.classList.add('copied')
        } catch {
            btn.textContent = 'copy failed'
        }
        setTimeout(() => {
            btn.textContent = 'copy'
            btn.classList.remove('copied')
        }, 2000)
    })
}

/* ------------------------------------------------------------------ */
/* The demo run: a faithful replay of `pulse release.yml`, looping     */
/* while the terminal is on screen.                                    */
/* ------------------------------------------------------------------ */

type Line = { t: string; c?: string; type?: boolean; pause?: number }

function demoLines(): Line[] {
    const pad = (n: number) => String(n).padStart(2, '0')
    const now = new Date()
    const ts =
        `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}` +
        `-${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`
    const report = `release-${ts}.md`
    return [
        { t: '$ pulse release.yml', type: true, pause: 500 },
        {
            t: `▶ workflow 'release' - 3 steps - ./.pulse/workflows/release.yml`,
            c: 't-dim',
            pause: 220,
        },
        { t: '  provider: OpenRouter', c: 't-dim', pause: 220 },
        { t: `  worktree: ~/.pulse/worktrees/${ts}-pulse`, c: 't-dim', pause: 220 },
        { t: `  report:   ${report}`, c: 't-dim', pause: 700 },

        { t: '[1/3] bump', pause: 320 },
        { t: '  goal: Cargo.toml and Cargo.lock are bumped, build passes', c: 't-dim', pause: 620 },
        { t: '  ↳ edit_file crates/core/Cargo.toml [ok]', c: 't-dim', pause: 620 },
        { t: '  ↳ bash cargo build --quiet [ok]', c: 't-dim', pause: 900 },
        { t: '  ✓ step done - goal reached in 1 attempt(s), $0.0021', c: 't-ok', pause: 700 },

        { t: '[2/3] changelog', pause: 320 },
        { t: '  goal: CHANGELOG.md has a 0.7.1 entry', c: 't-dim', pause: 620 },
        { t: "  ↳ grep '^## ' CHANGELOG.md [ok]", c: 't-dim', pause: 620 },
        { t: '  ↳ edit_file CHANGELOG.md [ok]', c: 't-dim', pause: 900 },
        { t: '  ✓ step done - goal reached in 1 attempt(s), $0.0017', c: 't-ok', pause: 700 },

        { t: '[3/3] commit', pause: 320 },
        { t: '  goal: one commit contains the release changes', c: 't-dim', pause: 620 },
        { t: '  ↳ bash git add -A && git commit -m "release 0.7.1" [ok]', c: 't-dim', pause: 900 },
        {
            t: '  ↻ goal not reached - retrying (attempt 2): no version in commit',
            c: 't-retry',
            pause: 620,
        },
        { t: '  ↳ bash git commit --amend -m "release 0.7.1" [ok]', c: 't-dim', pause: 900 },
        { t: '  ✓ step done - goal reached in 2 attempt(s), $0.0034', c: 't-ok', pause: 800 },

        { t: `✓ workflow complete - 3 steps, $0.0072 total`, c: 't-ok', pause: 300 },
        { t: `  report: ${report}`, c: 't-dim', pause: 5200 },
    ]
}

const runEl = document.getElementById('run')

function lineNode(l: Line): HTMLSpanElement {
    const el = document.createElement('span')
    el.className = `l${l.c ? ` ${l.c}` : ''}`
    if (l.type) {
        const p = document.createElement('span')
        p.className = 'p'
        p.textContent = l.t.slice(0, 1)
        el.appendChild(p)
        el.appendChild(document.createTextNode(l.t.slice(1)))
    } else {
        el.textContent = l.t
    }
    return el
}

function renderStatic(): void {
    if (!runEl) return
    runEl.textContent = ''
    for (const l of demoLines()) runEl.appendChild(lineNode(l))
    runEl.scrollTop = runEl.scrollHeight
}

let runToken = 0
let playing = false

async function playRun(): Promise<void> {
    const token = ++runToken
    if (!runEl) return
    runEl.textContent = ''
    for (const l of demoLines()) {
        if (token !== runToken) return
        const el = lineNode(l)
        let cur: HTMLElement | null = null
        if (l.type) {
            cur = document.createElement('span')
            cur.className = 'cur'
            el.appendChild(cur)
        }
        runEl.appendChild(el)
        runEl.scrollTop = runEl.scrollHeight
        if (l.type && cur) {
            // type the command out, cursor in tow
            const text = el.lastChild as Text
            const full = l.t.slice(1)
            text.textContent = ''
            for (const ch of full) {
                if (token !== runToken) return
                text.textContent += ch
                await sleep(28)
            }
            cur.remove()
        }
        await sleep(l.pause ?? 350)
    }
    if (token === runToken) playRun()
}

if (runEl) {
    if (REDUCED.matches) {
        renderStatic()
    } else {
        const io = new IntersectionObserver(
            (entries) => {
                for (const e of entries) {
                    if (e.isIntersecting && !playing) {
                        playing = true
                        void playRun()
                    } else if (!e.isIntersecting && playing) {
                        playing = false
                        runToken++ // stop the loop; a later entry restarts it
                    }
                }
            },
            { threshold: 0.25 },
        )
        io.observe(runEl)
    }
}

/* ------------------------------------------------------------------ */
/* Section reveals                                                     */
/* ------------------------------------------------------------------ */

if (!REDUCED.matches) {
    const io = new IntersectionObserver(
        (entries) => {
            for (const e of entries) {
                if (e.isIntersecting) {
                    e.target.classList.add('in')
                    io.unobserve(e.target)
                }
            }
        },
        { threshold: 0.12 },
    )
    for (const el of document.querySelectorAll('.reveal')) io.observe(el)
} else {
    for (const el of document.querySelectorAll('.reveal')) el.classList.add('in')
}
