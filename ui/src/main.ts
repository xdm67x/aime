import { marked } from 'marked'
import DOMPurify from 'dompurify'
import hljs from 'highlight.js/lib/common'

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
    prompt_tokens?: number
    completion_tokens?: number
    worktree_status?: string | null
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

// toast notifications: transient messages (worktree create/drop etc.)
function toast(msg: string) {
    const box = $('toasts')
    const t = document.createElement('div')
    t.className = 'toast'
    t.textContent = msg
    box.appendChild(t)
    requestAnimationFrame(() => t.classList.add('show'))
    setTimeout(() => {
        t.classList.remove('show')
        setTimeout(() => t.remove(), 300)
    }, 2000)
}

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
    ${(b.prompt_tokens ?? 0) + (b.completion_tokens ?? 0) > 0 ? `<span class="beat-tokens" title="${b.prompt_tokens ?? 0} prompt / ${b.completion_tokens ?? 0} completion tokens this session">${fmtTokens((b.prompt_tokens ?? 0) + (b.completion_tokens ?? 0))}</span>` : ''}
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
        const st = await invoke<string>('delete_beat', { id })
        if (selectedBeat === id) {
            selectedBeat = null
            $('run-title').textContent = ''
        }
        dropSession(id)
        refreshMain()
        toast(st)
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
        if (b.worktree_status) toast(b.worktree_status)
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
// links rendered from markdown must open in the default browser, not the webview
chat.addEventListener('click', (e) => {
    const a = (e.target as Element).closest('a[href^="http"]')
    if (!a) return
    e.preventDefault()
    ;(window as any).__TAURI__.opener.openUrl(a.getAttribute('href')).catch(() => {})
})

type SessionStatus = 'idle' | 'running' | 'done' | 'stopped' | 'error'

interface SessionState {
    id: number
    el: HTMLElement
    busy: boolean
    queue: { text: string; images: string[] }[]
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
    { who, time, text, images }: { who: string; time: string; text: string; images?: string[] },
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
    // pasted images render as thumbnails under the text
    for (const src of images ?? []) {
        const img = document.createElement('img')
        img.className = 'msg-image'
        img.src = src
        img.alt = 'pasted image'
        msg.appendChild(img)
    }
    view.appendChild(wrap)
    return wrap
}

interface ToolRowMsg {
    tool: string
    arguments: string
    result: string
    error: boolean
}

// compact one-line summary of a tool call's key arguments, shown in the
// collapsed header so commands are visible without expanding
function toolSummary(tool: string, argsJson: string): string {
    let args: Record<string, unknown> = {}
    try {
        const parsed = JSON.parse(argsJson)
        if (parsed && typeof parsed === 'object') args = parsed as Record<string, unknown>
    } catch {
        return ''
    }
    const first = (k: string) => (typeof args[k] === 'string' ? (args[k] as string) : '')
    switch (tool) {
        case 'bash': {
            const c = first('command')
            return c.replace(/\s+/g, ' ').trim()
        }
        case 'grep':
            return `${first('pattern')} → ${first('path') || '.'}`
        case 'read_file':
        case 'write_file':
        case 'edit_file':
            return first('path')
        default:
            return ''
    }
}

// tool calls render as light, collapsible full-width rows — not bubbles

// extension → highlight.js language (mirrors pulse-core's diff.rs subset)
function langForPath(path: string): string | undefined {
    const ext = path.includes('.') ? path.split('.').pop() : undefined
    const map: Record<string, string> = {
        rs: 'rust',
        ts: 'typescript',
        tsx: 'typescript',
        mts: 'typescript',
        js: 'javascript',
        jsx: 'javascript',
        mjs: 'javascript',
        cjs: 'javascript',
        py: 'python',
        go: 'go',
        c: 'c',
        h: 'c',
        cpp: 'cpp',
        cc: 'cpp',
        hpp: 'cpp',
        java: 'java',
        kt: 'kotlin',
        swift: 'swift',
        rb: 'ruby',
        php: 'php',
        cs: 'csharp',
        sh: 'bash',
        bash: 'bash',
        zsh: 'bash',
        json: 'json',
        yaml: 'yaml',
        yml: 'yaml',
        toml: 'ini',
        md: 'markdown',
        html: 'xml',
        htm: 'xml',
        css: 'css',
        scss: 'scss',
        sass: 'scss',
        sql: 'sql',
        xml: 'xml',
        svg: 'xml',
        lua: 'lua',
        zig: 'zig',
        dart: 'dart',
        ex: 'elixir',
        exs: 'elixir',
        hs: 'haskell',
        scala: 'scala',
        pl: 'perl',
        pm: 'perl',
        vue: 'xml',
    }
    return ext ? map[ext] : undefined
}

// syntax-highlight one line of code; falls back to plain escaped text
function hlLine(line: string, lang?: string): string {
    if (!lang) return esc(line)
    try {
        return hljs.highlight(line, { language: lang, ignoreIllegals: true }).value
    } catch {
        return esc(line)
    }
}

// git-style diff view for write_file / edit_file results: the backend appends
// a "\x1bDIFF\x1b\n" marker followed by +/- tagged lines; render each line
// color-coded and syntax-highlighted for the file's language
function renderDiff(path: string, diffText: string): string {
    const lang = langForPath(path)
    const rows = diffText
        .split('\n')
        .filter((l, i, a) => !(l === '' && i === a.length - 1))
        .map((line) => {
            const kind =
                line[0] === '+' ? 'add' : line[0] === '-' ? 'del' : line[0] === ' ' ? 'ctx' : 'meta'
            const body = kind === 'meta' ? line : line.slice(1)
            return `<div class="diff-line ${kind}"><code>${kind === 'meta' ? esc(body) : hlLine(body, lang)}</code></div>`
        })
        .join('')
    return `<div class="diff"><div class="diff-head">${esc(path)}</div>${rows}</div>`
}

// split a tool result into (plain summary, optional diff section); the
// backend (pulse-core tools.rs) appends "\n\x1bDIFF\x1b\n" + the diff body
const DIFF_MARKER = '\n\x1bDIFF\x1b\n'
function splitDiff(result: string): { text: string; diff: string | null } {
    const i = result.indexOf(DIFF_MARKER)
    if (i < 0) return { text: result, diff: null }
    return { text: result.slice(0, i), diff: result.slice(i + DIFF_MARKER.length) }
}

function renderTool(view: HTMLElement, t: ToolRowMsg) {
    const summary = esc(toolSummary(t.tool, t.arguments))
    const wrap = document.createElement('div')
    wrap.className = 'row-tool' + (t.error ? ' err' : '')
    wrap.innerHTML = `<button class="tool-head">
        <svg class="chev" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6"/></svg>
        <span class="tool-name">${esc(t.tool)}</span>
        <span class="tool-args">${summary}</span>
        <span class="tool-status">${t.error ? 'failed' : 'done'}</span>
      </button>
      <div class="tool-body" hidden></div>`
    const body = wrap.querySelector<HTMLElement>('.tool-body')!
    // file edits get a git-style syntax-colored diff view instead of raw text
    if ((t.tool === 'write_file' || t.tool === 'edit_file') && !t.error) {
        let path = ''
        let args: Record<string, unknown> = {}
        try {
            args = JSON.parse(t.arguments)
            path = typeof args.path === 'string' ? args.path : ''
        } catch {
            /* keep empty path */
        }
        const { text, diff } = splitDiff(t.result)
        body.innerHTML =
            (text ? `<pre>${esc(text)}</pre>` : '') + (diff ? renderDiff(path, diff) : '')
    } else {
        body.innerHTML = `<pre>${esc(t.arguments)}</pre><pre>${esc(t.result)}</pre>`
    }
    wrap.querySelector('.tool-head')!.addEventListener('click', () => {
        body.hidden = !body.hidden
        wrap.classList.toggle('open', !body.hidden)
    })
    view.appendChild(wrap)
    return wrap
}

function addMsg(
    s: SessionState,
    d: { who: string; time: string; text: string; images?: string[] },
) {
    const w = renderMsg(s.el, d)
    scrollView(s)
    return w
}
function addTool(s: SessionState, ev: ToolRowMsg) {
    const w = renderTool(s.el, ev)
    scrollView(s)
    return w
}

/* ---- queued prompts never appear in the chat — they show as removable
   pills above the prompt input and become messages only once picked up ---- */
const now = () => new Date().toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })

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
    } else if (ev.type === 'step_start') {
        // a workflow brick began — label the run with the pipeline progress
        s.liveLabel = `${ev.label} (${ev.index}/${ev.total}) · ${ev.model}`
        s.statusText = `${ev.label} ${ev.index}/${ev.total} · ${ev.model}`
        clearLive(s)
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
    renderQueuePills()
}

// pills for the focused session's queued prompts, sitting right on top of the
// prompt input; each removable via its ✕
function renderQueuePills() {
    const s = currentSess()
    const box = $('queue-pills')
    if (!s || !s.queue.length) {
        box.innerHTML = ''
        box.style.display = 'none'
        return
    }
    box.innerHTML = s.queue
        .map(
            (item, i) => `<span class="pill" title="${esc(item.text)}">
        <span class="pill-text">${item.images.length ? `🖼 ` : ''}${esc(item.text) || '(image)'}</span>
        <button class="pill-x" data-i="${i}" title="Remove from queue">✕</button>
      </span>`,
        )
        .join('')
    box.querySelectorAll('.pill-x').forEach((b) =>
        b.addEventListener('click', () => {
            s.queue.splice(Number((b as HTMLElement).dataset.i), 1)
            renderChips()
        }),
    )
    box.style.display = ''
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
                        result: (m as { raw_content?: string }).raw_content ?? m.content,
                        error: !!m.error,
                    })
                } else {
                    renderMsg(temp, {
                        who: m.role === 'user' ? 'user' : (m.model ?? 'assistant'),
                        time: t,
                        text: m.content,
                        images: (m as { images?: string[] }).images,
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
    loadUsageTotals(b.id)
    input.focus()
}

async function send() {
    const text = input.value.trim()
    if (!text && !pendingImages.length) return
    const s = currentSess()
    if (!s) return
    const item = { text, images: pendingImages }
    pendingImages = []
    renderPendingImages()
    input.value = ''
    autoGrow()
    renderMirror()
    renderCmdMenu()
    if (s.busy) {
        // this session's agent is still working — queue behind it; other
        // sessions run independently and are unaffected
        s.queue.push(item)
        renderChips()
        return
    }
    await startRun(s, item)
}

// take the next queued prompt (if any) and run it; resolves when the queue
// is drained and the session goes back to idle
async function startRun(s: SessionState, first: { text: string; images: string[] }) {
    const next = () => {
        const item = s.queue.shift()
        if (item === undefined) return null
        // the queued prompt becomes a visible message only once the run starts
        addMsg(s, { who: 'user', time: now(), text: item.text, images: item.images })
        return item
    }
    s.busy = true
    addMsg(s, { who: 'user', time: now(), text: first.text, images: first.images })
    refreshMain()
    let item: { text: string; images: string[] } | null = first
    try {
        while (item) {
            await runOne(s, item)
            if (s.status === 'stopped') break
            item = next()
        }
    } finally {
        s.busy = false
        if (s.status === 'running') s.status = 'done'
        refreshMain()
    }
}

async function runOne(s: SessionState, item: { text: string; images: string[] }) {
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
            usage: {
                model: string
                prompt_tokens: number
                completion_tokens: number
                cost_usd: number
            }[]
            cost_usd: number
            context_percent: number | null
            context_full: boolean
            new_beat_id?: number | null
        }>('run_task', { beatId: s.id, prompt: item.text, images: item.images })
        mergeRunUsage(s.id, r.usage)
        // only the focused session's usage shows under the input; other
        // sessions still record theirs (visible when you switch to them)
        if (selectedBeat === s.id) renderUsageStats(s.id, r.context_percent, r.model)
        // the sidebar row (cost + tokens) changed — refresh it
        loadBeats()
        s.liveLabel = `${r.model} · ${r.tier}`
        // the step event already sealed this text (base tier) — only seal
        // when the answer hasn't been rendered live yet (low tier, fallback)
        if (s.lastSealed === r.answer) clearLive(s)
        else sealLive(s, r.answer)
        s.status = 'done'
        s.statusText = r.context_full ? 'context full — run /compact' : ''
        // /compact produced a fresh summarized session: switch to it
        if (r.new_beat_id) {
            await loadBeats()
            const nb = beats.find((b) => b.id === r.new_beat_id)
            if (nb) openBeat(nb)
        }
    } catch (e) {
        clearLive(s)
        // a cancelled run (Escape) isn't an error — show it as a plain note
        const stopped = String(e).includes('stopped')
        // context-limit refusal: surface as a clear hint, not an error
        const ctxFull = String(e).includes('context limit reached')
        s.status = stopped ? 'stopped' : ctxFull ? 'idle' : 'error'
        s.statusText = stopped ? 'stopped' : ctxFull ? 'context full — run /compact' : String(e)
        const row = addMsg(s, {
            who: 'Pulse',
            time: now(),
            text: stopped
                ? 'Stopped.'
                : ctxFull
                  ? 'Session context limit reached — type /compact to open a new session holding only a summary of this one.'
                  : String(e),
        })
        if (!stopped && !ctxFull) row.querySelector('.msg')!.classList.add('error')
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
const input = $('input') as HTMLTextAreaElement

/* ---- slash commands: autocomplete + highlighted rendering ---- */
const SLASH_COMMANDS = [
    { name: '/compact', desc: 'Summarize this session into a fresh one' },
] as const

const cmdMenu = $('cmd-menu') as HTMLElement
const mirror = $('input-mirror') as HTMLElement
let cmdIndex = -1 // highlighted item in the autocomplete menu, -1 = none
let cmdMatches: (typeof SLASH_COMMANDS)[number][] = []

function currentCmdQuery(): string | null {
    // a command is being typed when the input starts with "/" and has no
    // space yet — matching is case-insensitive, prefix-based
    const v = input.value
    if (!v.startsWith('/') || /\s/.test(v)) return null
    return v.slice(1).toLowerCase()
}

function renderMirror() {
    const v = input.value
    if (v.startsWith('/') && !/\s/.test(v)) {
        // escape then wrap the whole token in a highlight span
        const esc = v.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
        mirror.innerHTML = `<span class="cmd">${esc}</span>`
    } else {
        mirror.textContent = v
    }
}

function renderCmdMenu() {
    const q = currentCmdQuery()
    if (q === null) {
        cmdMenu.style.display = 'none'
        cmdMatches = []
        cmdIndex = -1
        return
    }
    cmdMatches = SLASH_COMMANDS.filter((c) => c.name.slice(1).toLowerCase().startsWith(q))
    if (!cmdMatches.length) {
        cmdMenu.style.display = 'none'
        cmdIndex = -1
        return
    }
    if (cmdIndex >= cmdMatches.length) cmdIndex = cmdMatches.length - 1
    cmdMenu.innerHTML = cmdMatches
        .map(
            (c, i) =>
                `<div class="cmd-item${i === cmdIndex ? ' active' : ''}" data-i="${i}"><span class="cmd-name">${c.name}</span><span class="cmd-desc">${c.desc}</span></div>`,
        )
        .join('')
    cmdMenu.style.display = 'block'
}

function applyCommand() {
    if (cmdIndex >= 0 && cmdMatches[cmdIndex]) {
        input.value = cmdMatches[cmdIndex]!.name
    }
    cmdIndex = -1
    renderMirror()
    renderCmdMenu()
    autoGrow()
    input.focus()
}

input.addEventListener('input', () => {
    cmdIndex = -1
    renderMirror()
    renderCmdMenu()
    autoGrow()
})

// auto-grow: textarea height follows content, scrolling once max is reached
function autoGrow() {
    input.style.height = 'auto'
    input.style.height = Math.min(input.scrollHeight, 320) + 'px'
}

cmdMenu.addEventListener('mousedown', (e) => {
    // mousedown so the input keeps focus; click would blur it first
    e.preventDefault()
    const item = (e.target as HTMLElement).closest('.cmd-item') as HTMLElement | null
    if (item) {
        cmdIndex = Number(item.dataset.i)
        applyCommand()
    }
})

input.addEventListener('keydown', (e) => {
    if (cmdMenu.style.display === 'block') {
        if (e.key === 'ArrowDown') {
            e.preventDefault()
            cmdIndex = (cmdIndex + 1) % cmdMatches.length
            renderCmdMenu()
            return
        }
        if (e.key === 'ArrowUp') {
            e.preventDefault()
            cmdIndex = (cmdIndex - 1 + cmdMatches.length) % cmdMatches.length
            renderCmdMenu()
            return
        }
        if (e.key === 'Tab' || (e.key === 'Enter' && cmdIndex >= 0)) {
            e.preventDefault()
            applyCommand()
            return
        }
        if (e.key === 'Escape') {
            e.preventDefault()
            cmdMenu.style.display = 'none'
            return
        }
    }
    if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault()
        send()
    }
})

/* ---- usage stats line under the prompt input ---- */
const fmtTokens = (n: number) => (n >= 1000 ? (n / 1000).toFixed(1) + 'k' : String(n))
const fmtCost2 = (c: number) => (c > 0 ? '$' + (c < 0.01 ? c.toFixed(4) : c.toFixed(2)) : '')
const fmtCtx = (p: number) => (p < 10 ? p.toFixed(1) : Math.round(p)) + '%'

// session-wide per-model usage totals, cached per beat so switching sessions
// restores them instantly; refreshed from the DB on open and after each run
const usageCache = new Map<number, Map<string, { p: number; c: number; cost: number }>>()

function usageMapFor(beatId: number) {
    let m = usageCache.get(beatId)
    if (!m) {
        m = new Map()
        usageCache.set(beatId, m)
    }
    return m
}

// short display name: vendor prefix stripped, tail kept ("openai/gpt-4o" → gpt-4o)
const shortModel = (m: string) => m.split('/').pop() ?? m

function renderUsageStats(beatId: number, contextPercent: number | null, ctxModel: string) {
    const el = $('usage-stats') as HTMLElement
    const totals = usageMapFor(beatId)
    if (!totals.size) {
        el.style.display = 'none'
        return
    }
    const parts = Array.from(totals.entries())
        .sort(([, a], [, b]) => b.cost - a.cost)
        .map(([model, u]) => {
            const cost = fmtCost2(u.cost)
            return `<span class="usage-part" title="${esc(model)}: ${u.p} prompt / ${u.c} completion tokens${cost ? ` · ${cost}` : ''}"><b>${esc(shortModel(model))}</b>${fmtTokens(u.p)}▸${fmtTokens(u.c)}${cost ? `<i>${cost}</i>` : ''}</span>`
        })
    if (contextPercent !== null) {
        const pct = fmtCtx(contextPercent)
        const full = contextPercent >= 90
        parts.push(
            `<span class="usage-part ctx${full ? ' ctx-full' : ''}" title="Context window used by ${esc(ctxModel)}">ctx ${pct}</span>`,
        )
    }
    el.innerHTML = parts.join('')
    el.style.display = 'flex'
}

// merge one run's per-model usage into the session totals and re-render
function mergeRunUsage(
    beatId: number,
    usage: { model: string; prompt_tokens: number; completion_tokens: number; cost_usd: number }[],
) {
    const totals = usageMapFor(beatId)
    for (const u of usage) {
        const cur = totals.get(u.model) ?? { p: 0, c: 0, cost: 0 }
        cur.p += u.prompt_tokens
        cur.c += u.completion_tokens
        cur.cost += u.cost_usd
        totals.set(u.model, cur)
    }
}

// load a session's per-model totals from the DB (on open)
async function loadUsageTotals(beatId: number) {
    try {
        const rows = await invoke<
            {
                model: string
                prompt_tokens: number
                completion_tokens: number
                cost_usd: number
            }[]
        >('beat_usage_totals', { beatId })
        const totals = usageMapFor(beatId)
        totals.clear()
        for (const r of rows)
            totals.set(r.model, { p: r.prompt_tokens, c: r.completion_tokens, cost: r.cost_usd })
        if (selectedBeat === beatId) renderUsageStats(beatId, null, '')
    } catch (e) {
        console.error(e)
    }
}

input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) send()
})

/* ---- pasted clipboard images ---- */
let pendingImages: string[] = []

function renderPendingImages() {
    const strip = $('pending-images')
    strip.innerHTML = pendingImages
        .map(
            (src, i) =>
                `<span class="pending-img"><img src="${src}" alt="pasted image"><button data-i="${i}" title="Remove">✕</button></span>`,
        )
        .join('')
}
$('pending-images').onclick = (e) => {
    const btn = (e.target as HTMLElement).closest('button')
    if (btn) {
        pendingImages.splice(Number(btn.dataset.i), 1)
        renderPendingImages()
    }
}
input.addEventListener('paste', (e) => {
    const files = Array.from(e.clipboardData?.files ?? []).filter((f) =>
        f.type.startsWith('image/'),
    )
    if (!files.length) return
    e.preventDefault()
    for (const f of files.slice(0, 4)) {
        const reader = new FileReader()
        reader.onload = () => {
            if (typeof reader.result === 'string') {
                pendingImages.push(reader.result)
                renderPendingImages()
            }
        }
        reader.readAsDataURL(f)
    }
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
        // The saved value can't be read off the select here: it was applied
        // before any <option> existed, so the browser reset it to "". Restore
        // from the config loaded from the DB instead.
        const slot = sel.dataset.slot
        const saved = slot ? savedModelConfig[slot] : undefined
        sel.innerHTML =
            '<option value="">— select —</option>' +
            models.map((m) => `<option value="${esc(m.id)}">${esc(m.id)}</option>`).join('')
        if (saved) sel.value = saved
    }
}

/* ---- model routing config ---- */
// Last config loaded from the DB; populateSlots uses it to restore selections.
const savedModelConfig: Record<string, string> = {}
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
        await invoke('save_model_config', { cfg: config })
        setModelsStatus('Models saved.')
    } catch (e) {
        setModelsStatus(String(e), true)
    }
}

/* ---- workflows: ordered model bricks, edited and reordered in Settings --- */

type StepKind = 'agent' | 'ask' | 'reflexion'
interface WorkflowStep {
    kind: StepKind
    model: string
    prompt: string
    label: string
}
interface Workflow {
    id: string
    name: string
    description: string
    steps: WorkflowStep[]
}

const STEP_KIND_HINTS: Record<StepKind, string> = {
    agent: 'agentic loop with tools',
    ask: 'single completion, no tools',
    reflexion: 'critique the previous step',
}

let workflows: Workflow[] = []
let defaultWorkflowId = ''
let editingWorkflowId: string | null = null
// brick being dragged in the step list (index into the edited workflow)
let dragStepIndex: number | null = null

const wfStatus = (msg: string, err = false) => {
    const el = $('workflows-status')
    el.textContent = msg
    el.className = err ? 'err' : ''
}

async function loadWorkflows() {
    try {
        workflows = await invoke<Workflow[]>('list_workflows')
        defaultWorkflowId = await invoke<string>('get_default_workflow')
        renderWorkflowList()
    } catch (e) {
        wfStatus(String(e), true)
    }
}

function renderWorkflowList() {
    const box = $('workflow-list')
    box.innerHTML = workflows
        .map((w) => {
            const isDefault = w.id === defaultWorkflowId
            return `<div class="workflow-row" data-id="${esc(w.id)}">
                <input class="workflow-name" value="${esc(w.name)}" placeholder="Workflow name" />
                <span class="workflow-steps-count">${w.steps.length} steps</span>
                <label class="workflow-default">
                    <input type="radio" name="default-workflow" ${isDefault ? 'checked' : ''} />
                    default
                </label>
                <button class="wf-edit">${editingWorkflowId === w.id ? 'Close' : 'Edit'}</button>
                <button class="wf-remove">✕</button>
            </div>`
        })
        .join('')
    for (const row of Array.from(box.querySelectorAll('.workflow-row'))) {
        const id = (row as HTMLElement).dataset.id!
        const w = workflows.find((x) => x.id === id)
        if (!w) continue
        ;(row.querySelector('.workflow-name') as HTMLInputElement).oninput = (e) => {
            w.name = (e.target as HTMLInputElement).value
        }
        ;(row.querySelector('.workflow-default input') as HTMLInputElement).onchange = async () => {
            defaultWorkflowId = id
            try {
                await invoke('set_default_workflow', { id })
                wfStatus('Default workflow saved.')
            } catch (e) {
                wfStatus(String(e), true)
            }
            renderWorkflowList()
        }
        ;(row.querySelector('.wf-edit') as HTMLButtonElement).onclick = () => {
            editingWorkflowId = editingWorkflowId === id ? null : id
            renderWorkflowList()
            renderWorkflowEditor()
        }
        ;(row.querySelector('.wf-remove') as HTMLButtonElement).onclick = () => {
            workflows = workflows.filter((x) => x.id !== id)
            if (editingWorkflowId === id) {
                editingWorkflowId = null
                renderWorkflowEditor()
            }
            renderWorkflowList()
        }
    }
    renderWorkflowEditor()
}

function renderWorkflowEditor() {
    const box = $('workflow-editor')
    const w = workflows.find((x) => x.id === editingWorkflowId)
    if (!w) {
        box.style.display = 'none'
        box.innerHTML = ''
        return
    }
    box.style.display = ''
    box.innerHTML = `
        <div class="wf-meta">
            <input class="wf-desc" value="${esc(w.description)}" placeholder="Description (optional)" />
        </div>
        <p class="field-label">Steps — drag to reorder, each brick pins its own model</p>
        <div class="wf-steps">${w.steps.map((s, i) => stepBrickHtml(s, i, w.steps.length)).join('')}</div>
        <button class="btn wf-add-step">+ Add step</button>
    `
    const desc = box.querySelector('.wf-desc') as HTMLInputElement
    desc.oninput = () => {
        w.description = desc.value
    }
    for (const el of Array.from(box.querySelectorAll('.wf-step'))) {
        const i = Number((el as HTMLElement).dataset.index)
        ;(el.querySelector('.step-label') as HTMLInputElement).oninput = (e) => {
            w.steps[i].label = (e.target as HTMLInputElement).value
        }
        ;(el.querySelector('.step-kind') as HTMLSelectElement).onchange = (e) => {
            w.steps[i].kind = (e.target as HTMLSelectElement).value as StepKind
            renderWorkflowEditor()
        }
        ;(el.querySelector('.step-model') as HTMLSelectElement).onchange = (e) => {
            w.steps[i].model = (e.target as HTMLSelectElement).value
        }
        ;(el.querySelector('.step-prompt') as HTMLTextAreaElement).oninput = (e) => {
            w.steps[i].prompt = (e.target as HTMLTextAreaElement).value
        }
        ;(el.querySelector('.step-up') as HTMLButtonElement).onclick = () => {
            if (i === 0) return
            ;[w.steps[i - 1], w.steps[i]] = [w.steps[i], w.steps[i - 1]]
            renderWorkflowEditor()
        }
        ;(el.querySelector('.step-down') as HTMLButtonElement).onclick = () => {
            if (i === w.steps.length - 1) return
            ;[w.steps[i], w.steps[i + 1]] = [w.steps[i + 1], w.steps[i]]
            renderWorkflowEditor()
        }
        ;(el.querySelector('.step-remove') as HTMLButtonElement).onclick = () => {
            if (w.steps.length <= 1) return
            w.steps.splice(i, 1)
            renderWorkflowEditor()
        }
        // HTML5 drag & drop between bricks
        ;(el as HTMLElement).ondragstart = () => {
            dragStepIndex = i
        }
        ;(el as HTMLElement).ondragover = (e) => e.preventDefault()
        ;(el as HTMLElement).ondrop = (e) => {
            e.preventDefault()
            if (dragStepIndex === null || dragStepIndex === i) return
            const [moved] = w.steps.splice(dragStepIndex, 1)
            w.steps.splice(i, 0, moved)
            dragStepIndex = null
            renderWorkflowEditor()
        }
    }
    ;(box.querySelector('.wf-add-step') as HTMLButtonElement).onclick = () => {
        w.steps.push({ kind: 'agent', model: '', prompt: '', label: '' })
        renderWorkflowEditor()
    }
}

function stepBrickHtml(s: WorkflowStep, i: number, total: number) {
    const modelOptions =
        '<option value="">— model —</option>' +
        models
            .map(
                (m) =>
                    `<option value="${esc(m.id)}" ${m.id === s.model ? 'selected' : ''}>${esc(m.id)}</option>`,
            )
            .join('')
    return `<div class="wf-step" data-index="${i}" draggable="true">
        <span class="step-index" title="Drag to reorder">${i + 1}</span>
        <input class="step-label" value="${esc(s.label)}" placeholder="Label (e.g. Plan)" />
        <select class="step-kind">
            ${(Object.keys(STEP_KIND_HINTS) as StepKind[])
                .map((k) => `<option value="${k}" ${k === s.kind ? 'selected' : ''}>${k}</option>`)
                .join('')}
        </select>
        <select class="step-model">${modelOptions}</select>
        <span class="step-kind-hint">${STEP_KIND_HINTS[s.kind]}</span>
        <button class="step-up" ${i === 0 ? 'disabled' : ''}>↑</button>
        <button class="step-down" ${i === total - 1 ? 'disabled' : ''}>↓</button>
        <button class="step-remove">✕</button>
        <textarea class="step-prompt" placeholder="Custom prompt (optional) — {{prompt}} expands to the user message">${esc(s.prompt)}</textarea>
    </div>`
}

$('add-workflow').onclick = () => {
    const id = `wf-${Date.now().toString(36)}`
    workflows.push({
        id,
        name: 'New workflow',
        description: '',
        steps: [{ kind: 'agent', model: '', prompt: '', label: 'Plan' }],
    })
    editingWorkflowId = id
    renderWorkflowList()
}

$('save-workflows').onclick = async () => {
    try {
        await invoke('save_workflows', { wfs: workflows })
        wfStatus('Workflows saved.')
        await loadWorkflows()
    } catch (e) {
        wfStatus(String(e), true)
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
        for (const s of ['classifier', 'high', 'base', 'low'] as const) {
            savedModelConfig[s] = cfg[s] ?? ''
            slotSelect(s).value = cfg[s] ?? ''
        }
    } catch (e) {
        console.error(e)
    }
    try {
        models = await invoke('list_models')
        populateSlots()
    } catch (e) {
        console.error(e)
    }
    await loadWorkflows()
})()
