//! Local Mermaid CLI execution, PNG validation, and terminal-image caching.

use std::env;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use codex_utils_home_dir::find_codex_home;
use sha2::Digest;
use sha2::Sha256;

use crate::terminal_hyperlinks::HyperlinkLine;

const CACHE_VERSION: &str = "v2";
pub(crate) const RENDER_SCALE: u32 = 2;
const DISPLAY_WIDTH_SCALE_HALVES: u32 = 2;
const RENDER_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PNG_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PNG_DIMENSION: u32 = 8_192;
const MAX_PNG_PIXELS: u64 = 40_000_000;
const MAX_STDERR_BYTES: usize = 8 * 1024;
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
static CACHE_CLEANED: OnceLock<()> = OnceLock::new();

pub(crate) fn render(diagram: &str, width: usize) -> Result<Vec<HyperlinkLine>> {
    let codex_home = find_codex_home()?;
    let mmdc = resolve_mmdc_in(&codex_home).context("mmdc executable not found")?;
    let cache_root = codex_home.join("mermaid-cache");
    CACHE_CLEANED.get_or_init(|| crate::latex_render::expire_cached_pngs(&cache_root));
    let cache_dir = cache_root.join(CACHE_VERSION);
    fs::create_dir_all(&cache_dir)
        .with_context(|| format!("create Mermaid cache {}", cache_dir.display()))?;
    let theme = mermaid_theme();
    let browser = resolve_browser();
    let key = cache_key_with_identity(
        diagram,
        theme,
        &file_identity(&mmdc),
        browser.as_deref().map(file_identity),
    );
    let png = cache_dir.join(format!("{key}.png"));
    if validate_png(&png).is_err() {
        let _ = fs::remove_file(&png);
        compile_mermaid_with_browser(diagram, &mmdc, browser.as_deref(), &png, theme)?;
    }
    crate::latex_render::render_cached_display_png(&png, &key, width, DISPLAY_WIDTH_SCALE_HALVES)
}

fn resolve_mmdc_in(codex_home: &Path) -> Option<PathBuf> {
    if let Some(renderer) = env::var_os("CODEX_MERMAID_RENDERER")
        && let Some(path) = resolve_command(PathBuf::from(renderer))
    {
        return Some(path);
    }
    let bundled = codex_home
        .join("mermaid-runtime/node_modules/.bin")
        .join(if cfg!(windows) { "mmdc.cmd" } else { "mmdc" });
    if bundled.is_file() {
        return Some(bundled);
    }
    resolve_command(PathBuf::from(if cfg!(windows) {
        "mmdc.cmd"
    } else {
        "mmdc"
    }))
}

fn resolve_command(command: PathBuf) -> Option<PathBuf> {
    if command.components().count() > 1 {
        return command.is_file().then_some(command);
    }
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|directory| directory.join(&command))
        .find(|candidate| candidate.is_file())
}

fn compile_mermaid_with_browser(
    diagram: &str,
    mmdc: &Path,
    browser: Option<&Path>,
    output: &Path,
    theme: &str,
) -> Result<()> {
    let cache_dir = output
        .parent()
        .context("Mermaid cache output has no parent")?;
    let temp = tempfile::Builder::new()
        .prefix(".mermaid-")
        .tempdir_in(cache_dir)
        .context("create Mermaid render directory")?;
    let input = temp.path().join("diagram.mmd");
    let rendered = temp.path().join("diagram.png");
    let config = temp.path().join("config.json");
    fs::write(&input, diagram).context("write Mermaid source")?;
    let mermaid_config = serde_json::json!({
        "securityLevel": "strict",
        "htmlLabels": false,
        "flowchart": {
            "htmlLabels": false,
            "useMaxWidth": false
        },
        "themeVariables": {
            "fontFamily": "Menlo, Monaco, Courier New, monospace",
            "fontSize": "16px"
        }
    });
    fs::write(&config, serde_json::to_vec(&mermaid_config)?).context("write Mermaid config")?;
    let mut command = mmdc_command(mmdc);
    command
        .arg("-i")
        .arg(&input)
        .arg("-o")
        .arg(&rendered)
        .arg("-c")
        .arg(&config)
        .args(["-b", "transparent", "-s"])
        .arg(RENDER_SCALE.to_string())
        .args(["-t", theme]);
    let browser_config = browser_config(temp.path(), browser)?;
    command.arg("-p").arg(browser_config);
    run_with_timeout(&mut command)?;
    validate_png(&rendered)?;
    fs::rename(&rendered, output)
        .or_else(|err| validate_png(output).map_err(|_| err))
        .with_context(|| format!("atomically write Mermaid cache {}", output.display()))?;
    Ok(())
}

fn mmdc_command(mmdc: &Path) -> Command {
    #[cfg(target_os = "windows")]
    if mmdc
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "cmd" | "bat"))
    {
        let mut command = Command::new("cmd.exe");
        command.arg("/C").arg(mmdc);
        return command;
    }
    Command::new(mmdc)
}

fn browser_config(directory: &Path, browser: Option<&Path>) -> Result<PathBuf> {
    let path = directory.join("puppeteer.json");
    let mut config = serde_json::json!({
        "executablePath": browser,
        "args": [
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-default-apps",
            "--disable-domain-reliability",
            "--disable-sync",
            "--host-resolver-rules=MAP * ~NOTFOUND",
            "--metrics-recording-only",
            "--no-first-run"
        ]
    });
    if browser.is_none() {
        config
            .as_object_mut()
            .context("build Mermaid browser config")?
            .remove("executablePath");
    }
    fs::write(&path, serde_json::to_vec(&config)?).context("write Mermaid browser config")?;
    Ok(path)
}

fn resolve_browser() -> Option<PathBuf> {
    for variable in [
        "CODEX_MERMAID_BROWSER",
        "PUPPETEER_EXECUTABLE_PATH",
        "CHROME_PATH",
    ] {
        if let Some(path) = env::var_os(variable).map(PathBuf::from)
            && path.is_file()
        {
            return Some(path);
        }
    }

    #[cfg(target_os = "macos")]
    for path in [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
    ] {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }

    #[cfg(target_os = "linux")]
    for executable in [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
    ] {
        if let Some(path) = resolve_command(PathBuf::from(executable)) {
            return Some(path);
        }
    }

    #[cfg(target_os = "windows")]
    for root in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
        if let Some(root) = env::var_os(root) {
            let path = PathBuf::from(root).join("Google/Chrome/Application/chrome.exe");
            if path.is_file() {
                return Some(path);
            }
        }
    }

    None
}

fn run_with_timeout(command: &mut Command) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn mmdc")?;
    let stderr = child.stderr.take().context("capture mmdc stderr")?;
    let stderr_reader = std::thread::spawn(move || bounded_stderr(stderr));
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().context("wait for mmdc")? {
            let stderr = stderr_reader.join().unwrap_or_default();
            if status.success() {
                if mermaid_output_reports_error(&stderr) {
                    bail!(
                        "mmdc reported a Mermaid render error: {}",
                        String::from_utf8_lossy(&stderr)
                    );
                }
                return Ok(());
            }
            bail!(
                "mmdc exited with {status}: {}",
                String::from_utf8_lossy(&stderr)
            );
        }
        if started.elapsed() >= RENDER_TIMEOUT {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            let stderr = stderr_reader.join().unwrap_or_default();
            bail!(
                "mmdc timed out after {}s: {}",
                RENDER_TIMEOUT.as_secs(),
                String::from_utf8_lossy(&stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn mermaid_output_reports_error(stderr: &[u8]) -> bool {
    let stderr = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    ["syntax error", "parse error", "error in text"]
        .iter()
        .any(|marker| stderr.contains(marker))
}

fn bounded_stderr(mut stderr: impl Read) -> Vec<u8> {
    let mut captured = Vec::new();
    let mut buffer = [0; 1024];
    while let Ok(read) = stderr.read(&mut buffer) {
        if read == 0 {
            break;
        }
        let remaining = MAX_STDERR_BYTES.saturating_sub(captured.len());
        captured.extend_from_slice(&buffer[..read.min(remaining)]);
    }
    captured
}

fn validate_png(path: &Path) -> Result<()> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("read rendered Mermaid PNG {}", path.display()))?;
    if metadata.len() < PNG_SIGNATURE.len() as u64 || metadata.len() > MAX_PNG_BYTES {
        bail!("Mermaid PNG has an invalid size");
    }
    let bytes =
        fs::read(path).with_context(|| format!("read rendered Mermaid PNG {}", path.display()))?;
    if !bytes.starts_with(PNG_SIGNATURE) {
        bail!("Mermaid output is not a PNG");
    }
    let (width, height) = image::image_dimensions(path).context("read Mermaid PNG dimensions")?;
    if width == 0
        || height == 0
        || width > MAX_PNG_DIMENSION
        || height > MAX_PNG_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_PNG_PIXELS
    {
        bail!("Mermaid PNG dimensions exceed limits");
    }
    image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .context("decode Mermaid PNG")?;
    Ok(())
}

fn mermaid_theme() -> &'static str {
    let (red, green, blue) = crate::terminal_palette::default_bg().unwrap_or((24, 24, 24));
    if u32::from(red) * 299 + u32::from(green) * 587 + u32::from(blue) * 114 < 128_000 {
        "dark"
    } else {
        "default"
    }
}

fn file_identity(path: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let metadata = fs::metadata(&path).ok();
    let modified = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!(
        "{}:{}:{modified}",
        path.display(),
        metadata.map_or(0, |metadata| metadata.len())
    )
}

fn cache_key_with_identity(
    diagram: &str,
    theme: &str,
    mmdc_identity: &str,
    browser_identity: Option<String>,
) -> String {
    let mut digest = Sha256::new();
    digest.update(CACHE_VERSION);
    digest.update(format!(
        "mermaid-cli-11.16.0-png-scale-{RENDER_SCALE}-transparent-strict-html-labels-false-monospace"
    ));
    digest.update(theme);
    digest.update(mmdc_identity);
    digest.update(browser_identity.as_deref().unwrap_or("no-browser"));
    digest.update(diagram);
    format!("{:x}", digest.finalize())
}

#[cfg(test)]
#[path = "mermaid_runtime_tests.rs"]
mod tests;
