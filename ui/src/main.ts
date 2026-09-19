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
    project_id?: number | null
    project_name?: string | null
}

interface Project {
    id: number
    name: string
    path: string
    source: string
}

interface Model {
    id: string
    name: string
    context_length?: number
    pricing: { prompt: string; completion: string }
}

/* ---- projects (local dirs / gh clones) ---- */
let projects: Project[] = []
async function loadProjects() {
    try {
        projects = await invoke('list_projects')
    } catch (e) {
        console.error(e)
    }
    renderProjects()
}
function projectRow(p: Project) {
    return `<div class="run project" title="${esc(p.path)}" data-id="${p.id}">
    <span class="beat-name">${esc(p.name)}</span>
    <span class="proj-src">${p.source === 'github' ? 'gh' : 'local'}</span>
    <button class="beat-x beat-play" title="New session"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="6 3 20 12 6 21 6 3"/></svg></button>
    <button class="beat-x" title="Remove"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
  </div>`
}
function renderProjects() {
    $('projects').innerHTML = projects.length
        ? projects.map(projectRow).join('')
        : '<div class="proj-empty">none yet</div>'
}
$('projects').onclick = async (e) => {
    const el = (e.target as Element).closest<HTMLElement>('.project')
    if (!el) return
    const p = projects.find((x) => x.id === +el.dataset.id!)
    if (!p) return
    if ((e.target as Element).closest('.beat-play')) {
        // sessions spawned from a project run inside its directory —
        // reuse the beat modal so the task name/description can be set
        openBeatModal(p)
        return
    }
    if ((e.target as Element).closest('.beat-x')) {
        try {
            await invoke('remove_project', { id: p.id })
        } catch (err) {
            console.error(err)
        }
        loadProjects()
    }
}
$('new-project-btn').onclick = () => {
    $('project-modal-status').textContent = ''
    $('project-overlay').classList.add('open')
    ;($('project-repo') as HTMLInputElement).value = ''
    $('project-repo').focus()
}
const closeProjectModal = () => $('project-overlay').classList.remove('open')
$('close-project-modal').onclick = closeProjectModal
$('project-overlay').onclick = (e) => {
    if (e.target === $('project-overlay')) closeProjectModal()
}
async function addProject(cmd: 'clone_project' | 'add_project', arg: string) {
    const status = $('project-modal-status')
    if (!arg) return
    status.textContent = cmd === 'clone_project' ? 'Cloning…' : 'Adding…'
    try {
        await invoke(cmd, cmd === 'clone_project' ? { repo: arg } : { path: arg })
        closeProjectModal()
        loadProjects()
    } catch (err) {
        status.textContent = String(err)
    }
}
$('project-clone').onclick = () =>
    addProject('clone_project', ($('project-repo') as HTMLInputElement).value.trim())
$('project-repo').onkeydown = (e) => {
    if (e.key === 'Enter')
        addProject('clone_project', ($('project-repo') as HTMLInputElement).value.trim())
}
$('project-choose').onclick = async () => {
    // native folder picker → register the picked directory as a local project
    try {
        const path = await invoke<string | null>('pick_folder')
        if (path) await addProject('add_project', path)
    } catch (err) {
        $('project-modal-status').textContent = String(err)
    }
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
    ${b.project_name ? `<span class="proj-src">${esc(b.project_name)}</span>` : ''}
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
    openBeat(b)
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
        }
        dropSession(id)
        refreshMain()
    } catch (err) {
        console.error(err)
    }
    $('delete-overlay').classList.remove('open')
    loadBeats()
}

// project whose play button opened the modal (null for plain "new beat")
let modalProject: Project | null = null
function openBeatModal(p: Project | null) {
    modalProject = p
    $('beat-modal-status').textContent = ''
    $('beat-overlay').classList.add('open')
    ;($('beat-name') as HTMLInputElement).value = ''
    ;($('beat-desc') as HTMLTextAreaElement).value = ''
    ;($('beat-name') as HTMLInputElement).focus()
}
$('new-beat-btn').onclick = () => openBeatModal(null)
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
            projectId: modalProject?.id ?? null,
        })
        modalProject = null
        closeBeatModal()
        await loadBeats()
        openBeat(b)
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
loadProjects()
loadBeats()

/* ---- sessions: every beat keeps its own chat view + live state, so runs
   stream in parallel and switching sessions never loses messages ---- */
const chat = $('chat')

type SessionStatus = 'idle' | 'running' | 'done' | 'stopped' | 'error'

interface QueueItem {
    text: string
    el: HTMLElement
}

interface SessionState {
    id: number
    el: HTMLElement
    busy: boolean
    queue: QueueItem[]
    status: SessionStatus
    statusText: string
    loaded: boolean
    follow: boolean
    liveRow: HTMLElement | null
    liveText: string
    liveLabel: string
    lastSealed: string
    doneTimer: ReturnType<typeof setTimeout> | null
}

const sessions = new Map<number, SessionState>()

function sessFor(id: number): SessionState {
    let s = sessions.get(id)
    if (!s) {
        const el = document.createElement('div')
        el.className = 'session-view'
        chat.appendChild(el)
        const sess: SessionState = {
            id,
            el,
            busy: false,
            queue: [],
            status: 'idle',
            statusText: '',
            loaded: false,
            follow: true,
            liveRow: null,
            liveText: '',
            liveLabel: '',
            lastSealed: '',
            doneTimer: null,
        }
        el.addEventListener('scroll', () => {
            sess.follow = el.scrollTop + el.clientHeight >= el.scrollHeight - 60
        })
        sessions.set(id, sess)
        s = sess
    }
    return s
}
const currentSess = () => (selectedBeat ? sessFor(selectedBeat) : null)
const dropSession = (id: number) => {
    sessions.get(id)?.el.remove()
    sessions.delete(id)
}
function scrollView(s: SessionState) {
    if (currentSess() === s && s.follow) s.el.scrollTop = s.el.scrollHeight
}

function renderMsg(
    view: HTMLElement,
    { who, time, text }: { who: string; time: string; text: string },
) {
    const wrap = document.createElement('div')
    wrap.className = who === 'user' ? 'row-user' : 'row-agent'
    if (who === 'user') {
        wrap.innerHTML = `<div class="meta">User <span class="t">${time}</span></div><div class="msg user"></div>`
    } else {
        wrap.innerHTML = `<div class="meta agent"><span class="who">${esc(who)}</span> <span class="t">${time}</span></div><div class="msg"></div>`
    }
    const msg = wrap.querySelector('.msg')!
    if (who === 'user') msg.textContent = text
    // model output is untrusted → sanitize before injecting
    else msg.innerHTML = DOMPurify.sanitize(marked.parse(text, { async: false }))
    view.appendChild(wrap)
    return wrap
}

interface ToolRowMsg {
    tool: string
    arguments: string
    result: string
    error: boolean
}

// tool calls render as light, collapsible full-width rows — not bubbles
function renderTool(view: HTMLElement, t: ToolRowMsg) {
    const wrap = document.createElement('div')
    wrap.className = 'row-tool' + (t.error ? ' err' : '')
    wrap.innerHTML = `<button class="tool-head">
        <svg class="chev" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6"/></svg>
        <span class="tool-name">${esc(t.tool)}</span>
        <span class="tool-status">${t.error ? 'failed' : 'done'}</span>
      </button>
      <div class="tool-body" hidden>
        <pre>${esc(t.arguments)}</pre>
        <pre>${esc(t.result)}</pre>
      </div>`
    const body = wrap.querySelector<HTMLElement>('.tool-body')!
    wrap.querySelector('.tool-head')!.addEventListener('click', () => {
        body.hidden = !body.hidden
        wrap.classList.toggle('open', !body.hidden)
    })
    view.appendChild(wrap)
    return wrap
}

function addMsg(s: SessionState, d: { who: string; time: string; text: string }) {
    const w = renderMsg(s.el, d)
    scrollView(s)
    return w
}
function addTool(s: SessionState, ev: ToolRowMsg) {
    const w = renderTool(s.el, ev)
    scrollView(s)
    return w
}

/* ---- pending queue: queued prompts show as dimmed rows in the chat and
   are only promoted to real user messages once the session takes them ---- */
function renderQueueRow(s: SessionState, text: string): HTMLElement {
    const wrap = document.createElement('div')
    wrap.className = 'row-queued'
    wrap.innerHTML = `<div class="meta">Queued <span class="q-x" title="Remove from queue">✕</span></div><div class="msg user queued"></div>`
    wrap.querySelector('.msg')!.textContent = text
    wrap.querySelector('.q-x')!.addEventListener('click', () => {
        s.queue = s.queue.filter((q) => q.el !== wrap)
        wrap.remove()
        renderChips()
        refreshMain()
    })
    s.el.appendChild(wrap)
    scrollView(s)
    return wrap
}
// promote a queued row into a real user message (called when the run starts)
function promoteQueued(s: SessionState, item: QueueItem) {
    const now = () => new Date().toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
    const wrap = item.el
    if (!wrap.isConnected) {
        addMsg(s, { who: 'user', time: now(), text: item.text })
        return
    }
    wrap.className = 'row-user'
    wrap.innerHTML = `<div class="meta">User <span class="t">${now()}</span></div><div class="msg user"></div>`
    wrap.querySelector('.msg')!.textContent = item.text
    scrollView(s)
}

/* ---- streaming: live bubble fed by backend task-events, per session ---- */
function ensureLive(s: SessionState): HTMLElement {
    if (s.liveRow?.isConnected) return s.liveRow
    s.liveRow = document.createElement('div')
    s.liveRow.className = 'row-agent'
    s.liveRow.innerHTML = `<div class="meta agent"><span class="who"></span> <span class="t"></span></div><div class="msg"></div>`
    s.el.appendChild(s.liveRow)
    scrollView(s)
    return s.liveRow
}
function appendDelta(s: SessionState, t: string) {
    s.liveText += t
    const msg = ensureLive(s).querySelector('.msg')!
    msg.classList.add('streaming')
    // model output is untrusted → sanitize before injecting
    msg.innerHTML = DOMPurify.sanitize(marked.parse(s.liveText, { async: false }))
    scrollView(s)
}
function clearLive(s: SessionState) {
    s.liveRow?.remove()
    s.liveRow = null
    s.liveText = ''
    s.lastSealed = ''
}
// promote the streaming bubble into a finished message (create one when no
// bubble is live, e.g. the provider fell back to non-streaming)
function sealLive(s: SessionState, text?: string) {
    const t = text ?? s.liveText
    if (!t.trim()) {
        clearLive(s)
        return
    }
    const row = ensureLive(s)
    const msg = row.querySelector('.msg')!
    msg.classList.remove('streaming')
    msg.innerHTML = DOMPurify.sanitize(marked.parse(t, { async: false }))
    row.querySelector('.who')!.textContent = s.liveLabel || 'assistant'
    row.querySelector('.t')!.textContent = new Date().toLocaleTimeString([], {
        hour: 'numeric',
        minute: '2-digit',
    })
    s.liveRow = null
    s.liveText = ''
    s.lastSealed = t
    scrollView(s)
}

// task-events carry their beat id — route each one into that session's view
;(window as any).__TAURI__.event.listen('task-event', (e: any) => {
    const ev = e.payload
    const s = sessions.get(ev.beat_id)
    if (!s) return
    if (ev.type === 'start') {
        // no placeholder bubble — the input's progress border signals activity
        s.liveLabel = `${ev.model} · ${ev.tier}`
        s.statusText = `${ev.model} · ${ev.tier}`
        renderChips()
    } else if (ev.type === 'delta') {
        appendDelta(s, ev.text)
    } else if (ev.type === 'step') {
        sealLive(s, ev.text)
    } else if (ev.type === 'tool') {
        // narration streamed before a tool call isn't persisted — drop it
        clearLive(s)
        addTool(s, ev)
    }
})

/* ---- status chips above the input: one per session with activity ---- */
function renderChips() {
    const box = $('chips')
    const items = [...sessions.values()].filter((s) => s.status !== 'idle')
    box.innerHTML = items
        .map((s) => {
            const beat = beats.find((x) => x.id === s.id)
            const status =
                s.busy || s.queue.length
                    ? s.queue.length
                        ? `${s.queue.length} queued`
                        : s.statusText || 'running'
                    : s.status
            return `<button class="chip ${s.status}${currentSess() === s ? ' current' : ''}" data-id="${s.id}" title="${esc(s.statusText || s.status)}">
        <span class="dot"></span><span class="chip-name">${esc(beat?.name ?? 'beat')}</span>
        <span class="chip-status">${esc(status)}</span>
        ${s.busy ? '<span class="chip-x" title="Stop this session">✕</span>' : ''}
      </button>`
        })
        .join('')
    box.style.display = items.length ? '' : 'none'
}
$('chips').onclick = (e) => {
    const chip = (e.target as Element).closest<HTMLElement>('.chip')
    if (!chip) return
    const id = +chip.dataset.id!
    if ((e.target as Element).closest('.chip-x')) {
        invoke('cancel_task', { beatId: id }).catch(() => {})
        return
    }
    const b = beats.find((x) => x.id === id)
    if (b) openBeat(b)
}

/* ---- main classes + composer state follow the focused session ---- */
function refreshMain() {
    const s = currentSess()
    const main = $('main')
    main.classList.toggle('no-beat', !s)
    if (s) {
        main.classList.toggle('fresh', !s.el.children.length)
        document.querySelector('.input-wrap')!.classList.toggle('busy', s.busy)
    }
    renderChips()
}

// open a beat's session: its view is created once and kept alive afterwards,
// so switching back restores messages, streaming state and scroll position
async function openBeat(b: Beat) {
    selectedBeat = b.id
    const s = sessFor(b.id)
    $('run-title').textContent = b.name
    $('run-project').textContent = b.project_name ? `⌂ ${b.project_name}` : ''
    for (const el of Array.from(chat.children) as HTMLElement[])
        el.classList.toggle('active', el === s.el)
    if (!s.loaded) {
        s.loaded = true
        // load persisted history into a detached node first, then splice it in
        // ahead of any live rows (a run may already be streaming)
        const temp = document.createElement('div')
        try {
            const msgs = await invoke<
                {
                    role: string
                    content: string
                    model?: string
                    ts?: string
                    arguments?: string
                    error?: boolean
                }[]
            >('get_beat_messages', { id: b.id })
            for (const m of msgs) {
                const t = m.ts ? m.ts.slice(11, 16) : ''
                if (m.role === 'tool') {
                    renderTool(temp, {
                        tool: m.model ?? 'tool',
                        arguments: m.arguments ?? '',
                        result: m.content,
                        error: !!m.error,
                    })
                } else {
                    renderMsg(temp, {
                        who: m.role === 'user' ? 'user' : (m.model ?? 'assistant'),
                        time: t,
                        text: m.content,
                    })
                }
            }
        } catch (err) {
            console.error(err)
        }
        const kids = Array.from(temp.children)
        for (let i = kids.length - 1; i >= 0; i--) s.el.insertBefore(kids[i]!, s.el.firstChild)
        s.follow = true
    }
    s.el.scrollTop = s.el.scrollHeight
    renderBeats()
    refreshMain()
    input.focus()
}

async function send() {
    const text = input.value.trim()
    if (!text) return
    const s = currentSess()
    if (!s) return
    input.value = ''
    if (s.busy) {
        // this session's agent is still working — queue behind it; the prompt
        // shows as a dimmed row until the run actually picks it up
        s.queue.push({ text, el: renderQueueRow(s, text) })
        renderChips()
        return
    }
    await startRun(s, text)
}

// take the next queued prompt (if any) and run it; resolves when the queue
// is drained and the session goes back to idle
async function startRun(s: SessionState, first: string) {
    const next = () => {
        const item = s.queue.shift()
        if (!item) return null
        promoteQueued(s, item)
        return item.text
    }
    s.busy = true
    refreshMain()
    let text: string | null = first
    try {
        while (text) {
            await runOne(s, text)
            if (s.status === 'stopped') break
            text = next()
        }
    } finally {
        s.busy = false
        if (s.status === 'running') s.status = 'done'
        refreshMain()
    }
}

async function runOne(s: SessionState, text: string) {
    const now = () => new Date().toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
    clearLive(s)
    s.status = 'running'
    s.statusText = ''
    refreshMain()
    try {
        // work streams in live via task-events; this just finalizes the answer
        const r = await invoke<{
            tier: string
            model: string
            answer: string
        }>('run_task', { beatId: s.id, prompt: text })
        s.liveLabel = `${r.model} · ${r.tier}`
        // the step event already sealed this text (base tier) — only seal
        // when the answer hasn't been rendered live yet (low tier, fallback)
        if (s.lastSealed === r.answer) clearLive(s)
        else sealLive(s, r.answer)
        s.status = 'done'
        s.statusText = ''
    } catch (e) {
        clearLive(s)
        // a cancelled run (Escape) isn't an error — show it as a plain note
        const stopped = String(e).includes('stopped')
        s.status = stopped ? 'stopped' : 'error'
        s.statusText = stopped ? 'stopped' : String(e)
        const row = addMsg(s, { who: 'Pulse', time: now(), text: stopped ? 'Stopped.' : String(e) })
        if (!stopped) row.querySelector('.msg')!.classList.add('error')
    }
    // done/stopped chips clean themselves up; errors stay until looked at
    if (s.status === 'done' || s.status === 'stopped') {
        clearTimeout(s.doneTimer!)
        s.doneTimer = setTimeout(() => {
            if (s.status === 'done' || s.status === 'stopped') s.status = 'idle'
            renderChips()
        }, 8000)
    }
    renderChips()
}
const input = $('input') as HTMLInputElement
input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') send()
})

/* ---- settings ---- */
$('settings-btn').onclick = () => $('overlay').classList.add('open')
$('close-settings').onclick = () => $('overlay').classList.remove('open')
$('overlay').onclick = (e) => {
    if (e.target === $('overlay')) $('overlay').classList.remove('open')
}
window.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape') return
    // Escape stops the focused session only — background sessions keep running
    const s = currentSess()
    if (s && (s.busy || s.queue.length)) {
        // drop queued rows from the chat too, then stop the run
        for (const q of s.queue) q.el.remove()
        s.queue.length = 0
        invoke('cancel_task', { beatId: s.id }).catch(() => {})
        return
    }
    $('overlay').classList.remove('open')
    closeBeatModal()
    closeProjectModal()
    $('delete-overlay').classList.remove('open')
})

let models: Model[] = []
$('save').onclick = () => saveKey('openrouter', 'key', 'settings-status')
$('save-opencode').onclick = () => saveKey('opencode', 'key-opencode', 'settings-status-opencode')
$('save-litellm').onclick = async () => saveLiteLlm()
function saveKey(provider: string, inputId: string, statusId: string) {
    const status = $(statusId)
    try {
        invoke('save_api_key', { provider, key: ($(inputId) as HTMLInputElement).value })
        status.textContent = 'API key saved.'
        status.className = ''
        keyPlaceholder(provider)
    } catch (e) {
        status.textContent = String(e)
        status.className = 'err'
    }
}
const KEY_INPUTS: Record<string, string> = {
    openrouter: 'key',
    opencode: 'key-opencode',
    litellm: 'key-litellm',
}
async function keyPlaceholder(provider: string) {
    const inputId = KEY_INPUTS[provider]
    try {
        const key = await invoke<string>('get_api_key', { provider })
        if (key)
            ($(inputId) as HTMLInputElement).placeholder =
                `Saved: ••••${key.slice(-4)} (enter to replace)`
    } catch {
        /* first run: no key yet */
    }
}
async function urlPlaceholder(provider: string) {
    try {
        const url = await invoke<string | null>('get_base_url', { provider })
        if (url) ($('litellm-url') as HTMLInputElement).value = url
    } catch {
        /* first run: no url yet */
    }
}
async function saveLiteLlm() {
    const status = $('settings-status-litellm')
    try {
        await invoke('save_base_url', {
            provider: 'litellm',
            url: ($('litellm-url') as HTMLInputElement).value,
        })
        await invoke('save_api_key', {
            provider: 'litellm',
            key: ($('key-litellm') as HTMLInputElement).value,
        })
        status.textContent = 'Gateway URL and API key saved.'
        status.className = ''
        keyPlaceholder('litellm')
    } catch (e) {
        status.textContent = String(e)
        status.className = 'err'
    }
}
function populateSlots() {
    for (const sel of document.querySelectorAll<HTMLSelectElement>('select.model-slot')) {
        const saved = sel.value // preserve the currently chosen model
        sel.innerHTML =
            '<option value="">— select —</option>' +
            models.map((m) => `<option value="${esc(m.id)}">${esc(m.id)}</option>`).join('')
        sel.value = saved
    }
}

/* ---- model routing config ---- */
const setModelsStatus = (msg: string, err = false) => {
    const el = $('models-status')
    el.textContent = msg
    el.className = err ? 'err' : ''
}
const slotSelect = (slot: string) =>
    document.querySelector<HTMLSelectElement>(`select.model-slot[data-slot="${slot}"]`)!
$('save-models').onclick = async () => {
    const config = {
        classifier: slotSelect('classifier').value,
        high: slotSelect('high').value,
        base: slotSelect('base').value,
        low: slotSelect('low').value,
    }
    try {
        await invoke('save_model_config', { config })
        setModelsStatus('Models saved.')
    } catch (e) {
        setModelsStatus(String(e), true)
    }
}

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
    await keyPlaceholder('openrouter')
    await keyPlaceholder('opencode')
    await keyPlaceholder('litellm')
    await urlPlaceholder('litellm')
    try {
        const cfg = await invoke<{
            classifier: string
            high: string
            base: string
            low: string
        }>('get_model_config')
        for (const s of ['classifier', 'high', 'base', 'low'] as const)
            slotSelect(s).value = cfg[s] ?? ''
    } catch (e) {
        console.error(e)
    }
    try {
        models = await invoke('list_models')
        populateSlots()
    } catch (e) {
        console.error(e)
    }
})()
