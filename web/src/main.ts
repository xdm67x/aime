const RELEASES_API = 'https://api.github.com/repos/xdm67x/pulse/releases/latest'
const DOWNLOAD_BTN = document.getElementById('download-btn') as HTMLAnchorElement
const HINT = document.getElementById('download-hint') as HTMLElement

interface Asset {
    name: string
    browser_download_url: string
}

interface Release {
    assets: Asset[]
}

async function init(): Promise<void> {
    try {
        const res = await fetch(RELEASES_API)
        if (!res.ok) return
        const release = (await res.json()) as Release
        const tui = release.assets.find(
            (a) => a.name.startsWith('pulse-') && a.name.endsWith('aarch64-apple-darwin.tar.gz'),
        )
        if (tui) {
            DOWNLOAD_BTN.href = tui.browser_download_url
            HINT.textContent = `Downloading ${tui.name} — macOS Apple silicon.`
        }
    } catch {
        // Fallback: keep the releases page links.
    }
}

void init()
