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
    $('run-title').textContent = b.name
    $('run-project').textContent = b.project_name ? `⌂ ${b.project_name}` : ''
    $('main').classList.remove('no-beat')
    chat.replaceChildren()
    liveRow = null
    liveText = ''
    try {
        const msgs = await invoke<
            { role: string; content: string; model?: string; ts?: string; arguments?: string; error?: boolean }[]
        >('get_beat_messages', { id: b.id })
        for (const m of msgs) {
            const t = m.ts ? m.ts.slice(11, 16) : ''
            if (m.role === 'tool') {
                addTool({
                    tool: m.model ?? 'tool',
                    arguments: m.arguments ?? '',
                    result: m.content,
                    error: !!m.error,
                })
            } else {
                addMsg({
                    who: m.role === 'user' ? 'user' : m.model ?? 'assistant',
                    time: t,
                    text: m.content,
                })
            }
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
loadProjects()
loadBeats()

/* ---- chat (local only: sent messages append until beat is reloaded) ---- */
const chat = $('chat')
function addMsg({ who, time, text }: { who: string; time: string; text: string }) {
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
    chat.appendChild(wrap)
    chat.scrollTop = chat.scrollHeight
    return wrap
}

interface ToolRowMsg {
    tool: string
    arguments: string
    result: string
    error: boolean
}

// tool calls render as light, collapsible full-width rows — not bubbles
function addTool(s: ToolRowMsg) {
    const wrap = document.createElement('div')
    wrap.className = 'row-tool' + (s.error ? ' err' : '')
    wrap.innerHTML = `<button class="tool-head">
        <svg class="chev" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6"/></svg>
        <span class="tool-name">${esc(s.tool)}</span>
        <span class="tool-status">${s.error ? 'failed' : 'done'}</span>
      </button>
      <div class="tool-body" hidden>
        <pre>${esc(s.arguments)}</pre>
        <pre>${esc(s.result)}</pre>
      </div>`
    const body = wrap.querySelector<HTMLElement>('.tool-body')!
    wrap.querySelector('.tool-head')!.addEventListener('click', () => {
        body.hidden = !body.hidden
        wrap.classList.toggle('open', !body.hidden)
    })
    chat.appendChild(wrap)
    chat.scrollTop = chat.scrollHeight
    return wrap
}

/* ---- streaming: live bubble fed by backend task-events ---- */
let liveRow: HTMLElement | null = null
let liveText = ''
let liveLabel = ''
let lastSealed = ''
// a run in flight; further sends queue up until it finishes
let busy = false
const queue: { beatId: number; text: string }[] = []

function ensureLive(): HTMLElement {
    if (liveRow?.isConnected) return liveRow
    liveRow = document.createElement('div')
    liveRow.className = 'row-agent'
    liveRow.innerHTML = `<div class="meta agent"><span class="who"></span> <span class="t"></span></div><div class="msg"></div>`
    chat.appendChild(liveRow)
    chat.scrollTop = chat.scrollHeight
    return liveRow
}
function appendDelta(t: string) {
    liveText += t
    const msg = ensureLive().querySelector('.msg')!
    msg.classList.add('streaming')
    // model output is untrusted → sanitize before injecting
    msg.innerHTML = DOMPurify.sanitize(marked.parse(liveText, { async: false }))
    chat.scrollTop = chat.scrollHeight
}
function clearLive() {
    liveRow?.remove()
    liveRow = null
    liveText = ''
    lastSealed = ''
}
// promote the streaming bubble into a finished message (create one when no
// bubble is live, e.g. the provider fell back to non-streaming)
function sealLive(text?: string) {
    const t = text ?? liveText
    if (!t.trim()) {
        clearLive()
        return
    }
    const row = ensureLive()
    const msg = row.querySelector('.msg')!
    msg.classList.remove('streaming')
    msg.innerHTML = DOMPurify.sanitize(marked.parse(t, { async: false }))
    row.querySelector('.who')!.textContent = liveLabel || 'assistant'
    row.querySelector('.t')!.textContent = new Date().toLocaleTimeString([], {
        hour: 'numeric',
        minute: '2-digit',
    })
    liveRow = null
    liveText = ''
    lastSealed = t
    chat.scrollTop = chat.scrollHeight
}

;(window as any).__TAURI__.event.listen('task-event', (e: any) => {
    const ev = e.payload
    if (ev.type === 'start') {
        // no placeholder bubble — the input's progress border signals activity
        liveLabel = `${ev.model} · ${ev.tier}`
    } else if (ev.type === 'delta') {
        appendDelta(ev.text)
    } else if (ev.type === 'step') {
        sealLive(ev.text)
    } else if (ev.type === 'tool') {
        // narration streamed before a tool call isn't persisted — drop it
        clearLive()
        addTool(ev)
    }
})

async function send() {
    const text = input.value.trim()
    if (!text) return
    if (!selectedBeat) {
        addMsg({ who: 'Pulse', time: '', text: 'Select a beat first.' })
        return
    }
    input.value = ''
    const now = () => new Date().toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
    addMsg({ who: 'user', time: now(), text })
    // while the agent works, messages queue up and run after it finishes
    if (busy) {
        queue.push({ beatId: selectedBeat, text })
        return
    }
    busy = true
    document.querySelector('.input-wrap')!.classList.add('busy')
    try {
        await runOne(selectedBeat, text)
        while (queue.length) {
            const next = queue.shift()!
            await runOne(next.beatId, next.text)
        }
    } finally {
        busy = false
        document.querySelector('.input-wrap')!.classList.remove('busy')
        clearLive()
    }
}

async function runOne(beatId: number, text: string) {
    const now = () => new Date().toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
    $('main').classList.remove('fresh')
    clearLive()
    try {
        // work streams in live via task-events; this just finalizes the answer
        const r = await invoke<{
            tier: string
            model: string
            answer: string
        }>('run_task', { beatId, prompt: text })
        liveLabel = `${r.model} · ${r.tier}`
        // the step event already sealed this text (base tier) — only seal
        // when the answer hasn't been rendered live yet (low tier, fallback)
        if (lastSealed === r.answer) clearLive()
        else sealLive(r.answer)
    } catch (e) {
        clearLive()
        // a cancelled run (Escape) isn't an error — show it as a plain note
        const stopped = String(e).includes('stopped')
        const row = addMsg({ who: 'Pulse', time: now(), text: stopped ? 'Stopped.' : String(e) })
        if (!stopped) row.querySelector('.msg')!.classList.add('error')
    }
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
    // while a task runs, Escape stops it (queued sends are dropped too)
    if (busy) {
        queue.length = 0
        invoke('cancel_task').catch(() => {})
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
        await invoke('save_base_url', { provider: 'litellm', url: ($('litellm-url') as HTMLInputElement).value })
        await invoke('save_api_key', { provider: 'litellm', key: ($('key-litellm') as HTMLInputElement).value })
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
