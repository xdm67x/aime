//! Self-update against GitHub releases. CI publishes one asset per release —
//! `pulse-<tag>-aarch64-apple-darwin.tar.gz` containing the `pulse` binary —
//! so an update is: check the latest release, download its asset, and replace
//! the running executable in place. The repository is private, so both the
//! API call and the download need a token ([`token`]).

use crate::config;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Repository whose releases are checked.
const REPO: &str = "xdm67x/pulse";

/// Release asset name, kept in sync with `.github/workflows/release.yml` and
/// the mise install docs.
fn asset_name(tag: &str) -> String {
    format!("pulse-{tag}-aarch64-apple-darwin.tar.gz")
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    assets: Vec<GhAsset>,
}

/// A newer release ready to install: its tag (e.g. `v0.9.0`) and the download
/// URL of its `aarch64-apple-darwin` asset.
pub struct Release {
    pub tag: String,
    pub asset_url: String,
}

/// Token for the private repo: the `github` key in settings, falling back to
/// the usual GitHub token environment variables (mise clients already export
/// `MISE_GITHUB_TOKEN`).
fn token() -> Result<String, String> {
    let stored = config::get_api_key("github").unwrap_or(None);
    if let Some(t) = stored.filter(|t| !t.trim().is_empty()) {
        return Ok(t.trim().to_string());
    }
    for var in ["GITHUB_TOKEN", "MISE_GITHUB_TOKEN"] {
        if let Ok(t) = std::env::var(var) {
            if !t.trim().is_empty() {
                return Ok(t.trim().to_string());
            }
        }
    }
    Err("GitHub token required (the repository is private): \
        export GITHUB_TOKEN or MISE_GITHUB_TOKEN"
        .into())
}

fn client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}

/// The latest published release with its platform asset, or `None` when
/// GitHub reports no releases.
pub async fn latest_release() -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let token = token()?;
    let resp = client(Duration::from_secs(30))?
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "pulse")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Failed to reach GitHub: {e}"))?;
    match resp.status().as_u16() {
        200 => {}
        404 => return Ok(None),
        401 => {
            return Err(
                "GitHub rejected the token — export GITHUB_TOKEN or MISE_GITHUB_TOKEN with a token that can read the repo"
                    .into(),
            )
        }
        code => return Err(format!("GitHub returned HTTP {code}")),
    }
    let release: GhRelease = resp
        .json()
        .await
        .map_err(|e| format!("Failed to read the release: {e}"))?;
    let expected = asset_name(&release.tag_name);
    let asset_url = release
        .assets
        .iter()
        .find(|a| a.name == expected)
        .map(|a| a.browser_download_url.clone())
        .ok_or_else(|| format!("Release {} has no {expected} asset", release.tag_name))?;
    Ok(Some(Release {
        tag: release.tag_name,
        asset_url,
    }))
}

/// True when `tag` (e.g. `v0.9.0`) is strictly newer than `current`
/// (e.g. `0.8.0`): dotted numeric fields compared left to right.
pub fn is_newer(current: &str, tag: &str) -> bool {
    let cur = version_fields(current);
    let new = version_fields(tag);
    for i in 0..cur.len().max(new.len()) {
        let a = cur.get(i).copied().unwrap_or(0);
        let b = new.get(i).copied().unwrap_or(0);
        if b > a {
            return true;
        }
        if b < a {
            return false;
        }
    }
    false
}

fn version_fields(v: &str) -> Vec<u64> {
    v.trim_start_matches('v')
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap_or(0))
        .collect()
}

/// The latest release when it is newer than `current`, else `None`.
pub async fn check(current: &str) -> Result<Option<Release>, String> {
    let Some(release) = latest_release().await? else {
        return Ok(None);
    };
    Ok(is_newer(current, &release.tag).then_some(release))
}

/// Download the release asset, extract the `pulse` binary, and replace the
/// running executable in place. Returns the installed tag.
pub async fn apply(release: &Release) -> Result<String, String> {
    let bytes = download_asset(&release.asset_url).await?;
    let dir = downloads_dir()?;
    let archive = dir.join(asset_name(&release.tag));
    std::fs::write(&archive, &bytes)
        .map_err(|e| format!("Failed to write {}: {e}", archive.display()))?;
    let extract_dir = dir.join("extract");
    let _ = std::fs::remove_dir_all(&extract_dir);
    std::fs::create_dir_all(&extract_dir)
        .map_err(|e| format!("Failed to create {}: {e}", extract_dir.display()))?;
    let binary = extract_binary(&archive, &extract_dir)?;
    install_binary(&binary)?;
    let _ = std::fs::remove_dir_all(&extract_dir);
    let _ = std::fs::remove_file(&archive);
    crate::log::info(format!("updated to {}", release.tag));
    Ok(release.tag.clone())
}

async fn download_asset(url: &str) -> Result<Vec<u8>, String> {
    let token = token()?;
    let resp = client(Duration::from_secs(300))?
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "pulse")
        .send()
        .await
        .map_err(|e| format!("Failed to download the release: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Download failed: HTTP {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to download the release: {e}"))?;
    Ok(bytes.to_vec())
}

fn downloads_dir() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    let dir = Path::new(&home).join(".pulse").join("downloads");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Unpack the release tarball with the system `tar` (the app already shells
/// out to `gh` and `$EDITOR` elsewhere) and return the path of the `pulse`
/// binary inside it.
fn extract_binary(archive: &Path, dir: &Path) -> Result<PathBuf, String> {
    let output = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(dir)
        .output()
        .map_err(|e| format!("Failed to run tar: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "tar failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let binary = dir.join("pulse");
    binary
        .is_file()
        .then_some(binary)
        .ok_or_else(|| "Archive does not contain a `pulse` binary".to_string())
}

/// Replace the running executable with the downloaded binary.
fn install_binary(binary: &Path) -> Result<(), String> {
    let exe =
        std::env::current_exe().map_err(|e| format!("Failed to locate the running binary: {e}"))?;
    replace_at(binary, &exe)
}

/// Copy `binary` next to `exe`, make it executable, then swap it in with a
/// rename so the running process is never left with a half-written file.
fn replace_at(binary: &Path, exe: &Path) -> Result<(), String> {
    let staged = exe.with_file_name(format!(
        "{}.new",
        exe.file_name().unwrap_or_default().to_string_lossy()
    ));
    std::fs::copy(binary, &staged).map_err(|e| format!("Failed to stage the new binary: {e}"))?;
    set_executable(&staged)?;
    std::fs::rename(&staged, exe).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        format!("Failed to replace {}: {e}", exe.display())
    })
}

fn set_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::metadata(path)
        .map_err(|e| format!("Failed to stat the new binary: {e}"))?
        .permissions();
    let mut perms = perms;
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)
        .map_err(|e| format!("Failed to make the new binary executable: {e}"))
}

/// One-line TUI notice for a newer release. Check failures are logged, never
/// surfaced — a failed check must not disturb the app.
pub async fn startup_notice(current: &str) -> Option<String> {
    match check(current).await {
        Ok(Some(release)) => Some(format!(
            "→ pulse {} available — run `pulse update --apply` to update",
            release.tag
        )),
        Ok(None) => None,
        Err(e) => {
            crate::log::warn(format!("update check failed: {e}"));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asset_name() {
        assert_eq!(
            asset_name("v0.9.0"),
            "pulse-v0.9.0-aarch64-apple-darwin.tar.gz"
        );
    }

    #[test]
    fn test_is_newer() {
        assert!(is_newer("0.8.0", "v0.9.0"));
        assert!(is_newer("0.8.0", "v0.8.1"));
        assert!(is_newer("0.8", "v0.8.1"));
        assert!(is_newer("0.8.9", "v0.10.0"));
        assert!(!is_newer("0.8.0", "v0.8.0"));
        assert!(!is_newer("0.8.0", "v0.7.9"));
        assert!(!is_newer("0.8.10", "v0.8.9"));
    }

    #[test]
    fn test_version_fields_ignores_prefix_and_suffix() {
        assert_eq!(version_fields("0.8.0"), vec![0, 8, 0]);
        assert_eq!(version_fields("v1.2.3"), vec![1, 2, 3]);
        assert_eq!(version_fields("v1.2.3-beta"), vec![1, 2, 3]);
    }

    #[test]
    fn test_extract_binary() {
        let tmp = std::env::temp_dir().join(format!("pulse-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let src = tmp.join("pulse");
        std::fs::write(&src, b"fake binary").unwrap();
        let archive = tmp.join("pulse-v9.9.9-aarch64-apple-darwin.tar.gz");
        let status = std::process::Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&tmp)
            .arg("pulse")
            .status()
            .unwrap();
        assert!(status.success());
        let out = tmp.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let binary = extract_binary(&archive, &out).unwrap();
        assert_eq!(binary, out.join("pulse"));
        assert_eq!(std::fs::read(&binary).unwrap(), b"fake binary");
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_replace_at_swaps_atomically_and_cleans_up() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = std::env::temp_dir().join(format!("pulse-replace-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let exe = tmp.join("pulse");
        let incoming = tmp.join("incoming");
        std::fs::write(&exe, b"old").unwrap();
        std::fs::write(&incoming, b"new").unwrap();
        replace_at(&incoming, &exe).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert!(!exe.with_file_name("pulse.new").exists());
        let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111);
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
