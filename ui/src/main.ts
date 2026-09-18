import { marked } from 'marked'
import DOMPurify from 'dompurify'

const invoke = (window as any).__TAURI__.core.invoke as <T = any>(
    cmd: string,
    args?: Record<string, unknown>,
) => Promise<T>
const $ = (id: string) => document.getElementById(id) as HTMLElement

interface Beat {
    id: number
    name: string
    description?: string
    archived?: boolean
    cost_usd?: number
}

interface Model {
    id: string
    name: string
    context_length?: number
    pricing: { prompt: string; completion: string }
}

/* ---- beats (SQLite-backed) ---- */
let beats: Beat[] = []
let selectedBeat: number | null = null
const esc = (s: string) =>
    s.replace(
        /[&<>"']/g,
        (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!,
    )
const fmtCost = (c?: number) => (c && c > 0 ? '$' + (c < 0.01 ? c.toFixed(4) : c.toFixed(2)) : '')

async function loadBeats() {
    try {
        beats = await invoke('list_beats')
    } catch (e) {
        console.error(e)
    }
    renderBeats()
}
function beatRow(b: Beat) {
    const icon = (title: string, paths: string) =>
        `<button class="beat-x" title="${title}"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${paths}</svg></button>`
    const actions = b.archived
        ? icon(
              'Unarchive',
              '<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5v0a5.5 5.5 0 0 1-5.5 5.5H11"/>',
          ) + icon('Delete', '<path d="M18 6 6 18M6 6l12 12"/>')
        : icon('Archive', '<path d="M18 6 6 18M6 6l12 12"/>')
    return `<div class="run ${b.id === selectedBeat ? 'active' : ''} ${b.archived ? 'dim' : ''}" data-id="${b.id}">
    <span class="beat-name">${esc(b.name)}</span>
    ${b.archived ? '' : `<span class="beat-cost">${fmtCost(b.cost_usd)}</span>`}
    ${actions}
  </div>`
}
function renderBeats(q = ($('search') as HTMLInputElement).value.toLowerCase()) {
    const match = (b: Beat) => b.name.toLowerCase().includes(q)
    const active = beats.filter((b) => !b.archived && match(b))
    const archived = beats.filter((b) => b.archived && match(b))
    $('runs').innerHTML =
        active.map(beatRow).join('') +
        (archived.length
            ? '<div class="side-label" style="margin-top:16px">ARCHIVED</div>' +
              archived.map(beatRow).join('')
            : '')
}
$('runs').onclick = async (e) => {
    const el = (e.target as Element).closest<HTMLElement>('.run')
    if (!el) return
    const b = beats.find((x) => x.id === +el.dataset.id!)
    if (!b) return
    const action = (e.target as Element).closest<HTMLElement>('.beat-x')
    if (action) {
        try {
            if (action.title === 'Delete') {
                $('confirm-delete-name').textContent = `"${b.name}"`
                $('delete-overlay').dataset.beatId = String(b.id)
                $('delete-overlay').classList.add('open')
                return
            }
            await invoke('set_beat_archived', { id: b.id, archived: !b.archived })
        } catch (err) {
            console.error(err)
        }
        loadBeats()
        return
    }
    selectedBeat = b.id
    $('run-title').textContent = b.name
    $('main').classList.remove('no-beat')
    chat.replaceChildren()
    try {
        const msgs = await invoke<{ role: string; content: string; model?: string; ts?: string }[]>(
            'get_beat_messages',
            { id: b.id },
        )
        for (const m of msgs) {
            const t = m.ts ? m.ts.slice(11, 16) : ''
            addMsg({ who: m.role === 'user' ? 'user' : m.model ?? 'assistant', time: t, text: m.content })
        }
    } catch (err) {
        console.error(err)
    }
    $('main').classList.toggle('fresh', !chat.children.length)
    renderBeats()
}
$('delete-cancel').onclick = () => $('delete-overlay').classList.remove('open')
$('delete-overlay').onclick = (e) => {
    if (e.target === $('delete-overlay')) $('delete-overlay').classList.remove('open')
}
$('delete-confirm').onclick = async () => {
    const id = +$('delete-overlay').dataset.beatId!
    try {
        await invoke('delete_beat', { id })
        if (selectedBeat === id) {
            selectedBeat = null
            $('run-title').textContent = ''
            $('main').classList.add('no-beat')
            $('main').classList.remove('fresh')
        }
    } catch (err) {
        console.error(err)
    }
    $('delete-overlay').classList.remove('open')
    loadBeats()
}

$('new-beat-btn').onclick = () => {
    $('beat-modal-status').textContent = ''
    $('beat-overlay').classList.add('open')
    ;($('beat-name') as HTMLInputElement).value = ''
    ;($('beat-desc') as HTMLTextAreaElement).value = ''
    $('beat-name').focus()
}
const closeBeatModal = () => $('beat-overlay').classList.remove('open')
$('close-beat-modal').onclick = closeBeatModal
$('beat-cancel').onclick = closeBeatModal
$('beat-overlay').onclick = (e) => {
    if (e.target === $('beat-overlay')) closeBeatModal()
}
async function createBeat() {
    const name = ($('beat-name') as HTMLInputElement).value.trim()
    if (!name) return
    try {
        const b = await invoke<Beat>('create_beat', {
            name,
            description: ($('beat-desc') as HTMLTextAreaElement).value,
        })
        selectedBeat = b.id
        $('run-title').textContent = b.name
        chat.replaceChildren()
        $('main').classList.remove('no-beat')
        $('main').classList.add('fresh')
        closeBeatModal()
        await loadBeats()
    } catch (err) {
        $('beat-modal-status').textContent = String(err)
        $('beat-modal-status').classList.add('err')
    }
}
$('beat-create').onclick = createBeat
$('beat-name').onkeydown = (e) => {
    if (e.key === 'Enter') createBeat()
}
$('beat-desc').onkeydown = (e) => {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) createBeat()
}
$('search').oninput = () => renderBeats()
loadBeats()

/* ---- chat (local only: sent messages append until beat is reloaded) ---- */
const chat = $('chat')
// shared with the input mirror below
const withMentions = (t: string) =>
    esc(t).replace(/@([\w./-]+)/g, '<span class="mention">@$1</span>')
function addMsg({ who, time, text }: { who: string; time: string; text: string }) {
    const wrap = document.createElement('div')
    wrap.className = who === 'user' ? 'row-user' : 'row-agent'
    if (who === 'user') {
        wrap.innerHTML = `<div class="meta">User <span class="t">${time}</span></div><div class="msg user"></div>`
    } else {
        wrap.innerHTML = `<div class="meta agent">${who} <span class="t">${time}</span></div><div class="msg"></div>`
    }
    const msg = wrap.querySelector('.msg')!
    if (who === 'user') msg.innerHTML = withMentions(text)
    // model output is untrusted → sanitize before injecting
    else msg.innerHTML = DOMPurify.sanitize(marked.parse(text, { async: false }))
    chat.appendChild(wrap)
    chat.scrollTop = chat.scrollHeight
    return wrap
}

async function send() {
    const text = input.value.trim()
    if (!text) return
    input.value = ''
    paintMentions() // value assignment doesn't fire 'input'
    closeAt()
    $('main').classList.remove('fresh')
    addMsg({
        who: 'user',
        time: new Date().toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' }),
        text,
    })
    // each @model segment (until the next @ or end) goes to that model
    const matches = [...text.matchAll(/@([\w./-]+)/g)]
    if (matches.length >= 2) {
        // council: all mentioned models deliberate together in a sub-context,
        // seeded with a summary of this beat's prior context
        if (!selectedBeat) {
            addMsg({ who: 'council', time: '', text: 'Select a beat first — the council needs its context.' })
            return
        }
        const prompt = matches
            .map((m, i) =>
                text
                    .slice(m.index + m[0].length, i + 1 < matches.length ? matches[i + 1].index : text.length)
                    .trim(),
            )
            .filter(Boolean)
            .join(' ')
        if (!prompt) return
        const convening = addMsg({
            who: 'council',
            time: new Date().toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' }),
            text: 'Convening…',
        })
        convening.querySelector('.msg')!.classList.add('thinking')
        try {
            const r = await invoke<{ transcript: { agent_id: string; content: string }[]; artifact: string }>(
                'run_council',
                { beatId: selectedBeat, prompt, models: matches.map((m) => m[1]) },
            )
            convening.remove()
            const t = new Date().toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
            for (const turn of r.transcript)
                if (turn.agent_id !== 'user') addMsg({ who: turn.agent_id, time: t, text: turn.content })
            addMsg({ who: 'council', time: t, text: r.artifact })
        } catch (e) {
            convening.querySelector('.msg')!.textContent = String(e)
            convening.querySelector('.msg')!.classList.remove('thinking')
        }
        return
    }
    for (const [i, m] of matches.entries()) {
        const start = m.index + m[0].length
        const stop = i + 1 < matches.length ? matches[i + 1].index : text.length
        const body = text.slice(start, stop).trim()
        if (!body) continue
        const wrap = addMsg({
            who: m[1],
            time: new Date().toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' }),
            text: 'Thinking…',
        })
        const bubble = wrap.querySelector('.msg')!
        bubble.classList.add('thinking')
        try {
            const reply = await invoke<string>('send_message', {
                beatId: selectedBeat,
                model: m[1],
                content: body,
            })
            bubble.innerHTML = DOMPurify.sanitize(marked.parse(reply, { async: false }))
            bubble.classList.remove('thinking')
        } catch (e) {
            bubble.textContent = String(e)
            bubble.classList.remove('thinking')
        }
    }
}
const input = $('input') as HTMLInputElement
const mirror = $('mirror')
const paintMentions = () => {
    mirror.innerHTML = withMentions(input.value) + '\u00a0'
    mirror.scrollLeft = input.scrollLeft
}
input.addEventListener('input', paintMentions)
input.addEventListener('scroll', paintMentions)
paintMentions()
const popup = $('at-popup')
let atItems: { el: HTMLElement; id: string }[] = []
let atIdx = 0

const atToken = () => {
    const m = /@([\w./-]*)$/.exec(input.value)
    return m ? m[1] : null
}
const closeAt = () => {
    popup.hidden = true
    atItems = []
}
function renderAt(q: string) {
    const matches = models
        .filter(
            (m) =>
                m.id.toLowerCase().includes(q) || m.name.toLowerCase().includes(q),
        )
        .slice(0, 8)
    if (!matches.length) return closeAt()
    atIdx = Math.min(atIdx, matches.length - 1)
    atItems = matches.map((m) => {
        const el = document.createElement('div')
        el.className = 'at-item'
        el.innerHTML = `<span class="at-id">@${esc(m.id)}</span><span class="at-cost">${price(m.pricing.prompt)}/M in</span>`
        el.onclick = () => pickAt(m.id)
        return { el, id: m.id }
    })
    popup.replaceChildren(...atItems.map((x) => x.el))
    updateAtActive()
    popup.hidden = false
}
const updateAtActive = () => {
    atItems.forEach((x, i) => x.el.classList.toggle('active', i === atIdx))
    atItems[atIdx]?.el.scrollIntoView({ block: 'nearest' })
}
function pickAt(id: string) {
    input.value = input.value.replace(/@[\w./-]*$/, `@${id} `)
    paintMentions() // value assignment doesn't fire 'input'
    closeAt()
    input.focus()
}
let atLoading = false
input.addEventListener('input', () => {
    const tok = atToken()
    if (tok === null) return closeAt()
    if (!models.length && !atLoading) {
        atLoading = true
        invoke('list_models')
            .then((m) => {
                models = m
                if (atToken() !== null) renderAt(atToken()!)
            })
            .catch((e) => console.error(e))
            .finally(() => (atLoading = false))
    }
    atIdx = 0
    renderAt(tok.toLowerCase())
})

$('input').onkeydown = (e) => {
    if (!popup.hidden) {
        if (e.key === 'ArrowDown') {
            atIdx = (atIdx + 1) % atItems.length
            updateAtActive()
            return e.preventDefault()
        }
        if (e.key === 'ArrowUp') {
            atIdx = (atIdx - 1 + atItems.length) % atItems.length
            updateAtActive()
            return e.preventDefault()
        }
        if (e.key === 'Enter' || e.key === 'Tab') {
            return e.preventDefault(), pickAt(atItems[atIdx].id)
        }
        if (e.key === 'Escape') return closeAt()
    }
    if (e.key === 'Enter') send()
}
input.addEventListener('blur', () => setTimeout(closeAt, 150))

/* ---- settings ---- */
const setStatus = (msg: string, err = false) => {
    $('settings-status').textContent = msg
    $('settings-status').className = err ? 'err' : ''
}

$('settings-btn').onclick = () => $('overlay').classList.add('open')
$('close-settings').onclick = () => $('overlay').classList.remove('open')
$('overlay').onclick = (e) => {
    if (e.target === $('overlay')) $('overlay').classList.remove('open')
}
window.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape') return
    $('overlay').classList.remove('open')
    closeBeatModal()
    $('delete-overlay').classList.remove('open')
})

let models: Model[] = []
function price(x: string) {
    const p = parseFloat(x)
    if (!p) return '<span class="free">Free</span>'
    return '$' + (p * 1e6).toFixed(p * 1e6 < 10 ? 3 : 2)
}
function render() {
    const q = ($('filter') as HTMLInputElement).value.toLowerCase()
    $('rows').innerHTML = models
        .filter((m) => m.id.toLowerCase().includes(q) || m.name.toLowerCase().includes(q))
        .map(
            (
                m,
            ) => `<tr><td>${m.id}</td><td>${m.name}</td><td class="num">${m.context_length ?? '—'}</td>
      <td class="num">${price(m.pricing.prompt)}</td><td class="num">${price(m.pricing.completion)}</td></tr>`,
        )
        .join('')
}
$('save').onclick = async () => {
    try {
        await invoke('save_api_key', { key: ($('key') as HTMLInputElement).value })
        setStatus('API key saved.')
    } catch (e) {
        setStatus(String(e), true)
    }
}
$('load').onclick = async () => {
    const load = $('load') as HTMLButtonElement
    load.disabled = true
    setStatus('Loading models…')
    try {
        models = await invoke('list_models')
        setStatus(`${models.length} models from OpenRouter.`)
        render()
    } catch (e) {
        setStatus(String(e), true)
    }
    load.disabled = false
}
$('filter').oninput = render

/* ---- settings tabs ---- */
const showTab = (name: string) => {
    for (const b of document.querySelectorAll<HTMLButtonElement>('#settings-tabs button'))
        b.classList.toggle('active', b.dataset.tab === name)
    for (const p of document.querySelectorAll('.tab-pane'))
        (p as HTMLElement).style.display = p.id === `pane-${name}` ? '' : 'none'
}
for (const b of document.querySelectorAll<HTMLButtonElement>('#settings-tabs button'))
    b.onclick = () => showTab(b.dataset.tab!)

/* ---- appearance: theme ---- */
const applyTheme = (t: string) => {
    document.documentElement.dataset.theme = t
    localStorage.setItem('theme', t)
    for (const b of document.querySelectorAll<HTMLButtonElement>('#theme-seg button'))
        b.classList.toggle('active', b.dataset.theme === t)
}
for (const b of document.querySelectorAll<HTMLButtonElement>('#theme-seg button'))
    b.onclick = () => applyTheme(b.dataset.theme!)
applyTheme(localStorage.getItem('theme') ?? 'dark')

;(async () => {
    const key = await invoke<string>('get_api_key')
    if (key)
        ($('key') as HTMLInputElement).placeholder =
            `Saved: ••••${key.slice(-4)} (enter to replace)`
})()
