const INSTALL_CMD = 'mise use -g "github:xdm67x/pulse@latest"'
const COPY_BTN = document.getElementById('copy-btn') as HTMLButtonElement
const HINT = document.getElementById('install-hint') as HTMLElement

const DEFAULT_HINT = HINT.textContent

COPY_BTN.addEventListener('click', async () => {
    try {
        await navigator.clipboard.writeText(INSTALL_CMD)
        COPY_BTN.textContent = 'Copied!'
        HINT.textContent = 'Command copied to clipboard.'
    } catch {
        HINT.textContent = `Copy and run: ${INSTALL_CMD}`
    }
    setTimeout(() => {
        COPY_BTN.textContent = 'Copy'
        HINT.textContent = DEFAULT_HINT
    }, 2000)
})
