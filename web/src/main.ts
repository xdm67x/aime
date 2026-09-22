const RELEASES_API = 'https://api.github.com/repos/xdm67x/pulse/releases/latest'
const DOWNLOAD_BTN = document.getElementById('download-btn') as HTMLAnchorElement
const HINT = document.getElementById('download-hint') as HTMLElement

const VSIX_TARGETS = ['darwin-arm64', 'linux-x64', 'win32-x64']

interface Asset {
    name: string
    browser_download_url: string
}

interface Release {
    assets: Asset[]
}

function wireVsixDownloads(release: Release): void {
    for (const target of VSIX_TARGETS) {
        const link = document.getElementById(`vsix-${target}`) as HTMLAnchorElement | null
        if (!link) continue
        const vsix = release.assets.find(
            (a) => a.name.endsWith(`-${target}.vsix`) && a.name.startsWith('pulse-vscode-'),
        )
        if (vsix) link.href = vsix.browser_download_url
    }
}

async function init(): Promise<void> {
    try {
        const res = await fetch(RELEASES_API)
        if (!res.ok) return
        const release = (await res.json()) as Release
        const dmg = release.assets.find((a) => a.name.endsWith('.dmg'))
        if (dmg) {
            DOWNLOAD_BTN.href = dmg.browser_download_url
            HINT.textContent = `Downloading ${dmg.name} — Apple silicon & Intel.`
        }
        wireVsixDownloads(release)
    } catch {
        // Fallback: keep the releases page links.
    }
}

void init()
