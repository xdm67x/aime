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

async function wireDownload(): Promise<void> {
    try {
        const res = await fetch(RELEASES_API)
        if (!res.ok) return
        const release = (await res.json()) as Release
        const dmg = release.assets.find((a) => a.name.endsWith('.dmg'))
        if (dmg) {
            DOWNLOAD_BTN.href = dmg.browser_download_url
            HINT.textContent = `Downloading ${dmg.name} — Apple silicon & Intel.`
        }
    } catch {
        // Fallback: keep the releases page link.
    }
}

void wireDownload()
