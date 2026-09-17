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
    $('main').classList.add('fresh')
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
function addMsg({ who, time, text }: { who: string; time: string; text: string }) {
    const wrap = document.createElement('div')
    wrap.className = who === 'user' ? 'row-user' : 'row-agent'
    if (who === 'user') {
        wrap.innerHTML = `<div class="meta">User <span class="t">${time}</span></div><div class="msg user"></div>`
    } else {
        wrap.innerHTML = `<div class="meta agent">${who} <span class="t">${time}</span></div><div class="msg"></div>`
    }
    wrap.querySelector('.msg')!.textContent = text
    chat.appendChild(wrap)
    chat.scrollTop = chat.scrollHeight
}

function send() {
    const input = $('input') as HTMLInputElement
    const text = input.value.trim()
    if (!text) return
    addMsg({
        who: 'user',
        time: new Date().toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' }),
        text,
    })
    input.value = ''
}
$('input').onkeydown = (e) => {
    if (e.key === 'Enter') send()
}

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
