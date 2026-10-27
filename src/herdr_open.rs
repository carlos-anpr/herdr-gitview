//! Acción `open` del plugin: abre el visor como popup sobre el panel activo, en la carpeta de ese panel.
//! Sustituye a `herdr/open.sh` para que la acción funcione igual en Windows, donde herdr no pasa por un shell.

use anyhow::{Context, Result};
use std::process::Command;

/// Entrypoint del manifiesto para esta plataforma (en Windows el panel lanza el .exe sin `sh`).
const ENTRYPOINT: &str = if cfg!(windows) { "viewer-windows" } else { "viewer" };

pub fn run() -> Result<i32> {
    let bin = std::env::var("HERDR_BIN_PATH").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "herdr".into());
    let ctx = std::env::var("HERDR_PLUGIN_CONTEXT_JSON").unwrap_or_default();
    let status = Command::new(&bin)
        .args(["plugin", "pane", "open", "--plugin", "local.gitview", "--entrypoint", ENTRYPOINT])
        .args(["--placement", "popup", "--width", "94%", "--height", "92%"])
        .arg("--env")
        .arg(format!("HERDR_PLUGIN_CONTEXT_JSON={ctx}"))
        .status()
        .with_context(|| format!("no se pudo lanzar «{bin}»"))?;
    Ok(status.code().unwrap_or(1))
}
