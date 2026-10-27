//! Helpers multiplataforma: rutas del sistema, abrir URL/ficheros y lanzar subprocesos.
//! Lo específico de Windows va detrás de `#[cfg(windows)]`; en Linux/macOS todo sigue igual.
//! PROPIETARIO: OpenCode.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Variable de entorno no vacía.
fn nonempty(key: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(key).filter(|v| !v.is_empty())
}

/// `$HOME` si no está vacío; en Windows, si no, `%USERPROFILE%`.
pub fn home_dir() -> Option<PathBuf> {
    if let Some(home) = nonempty("HOME") {
        return Some(PathBuf::from(home));
    }
    #[cfg(windows)]
    if let Some(profile) = nonempty("USERPROFILE") {
        return Some(PathBuf::from(profile));
    }
    None
}

/// `$XDG_CONFIG_HOME` o `home/.config` (también en Windows, como opencode).
pub fn config_dir() -> Option<PathBuf> {
    if let Some(xdg) = nonempty("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg));
    }
    Some(home_dir()?.join(".config"))
}

/// `$XDG_CACHE_HOME`; en Windows `%LOCALAPPDATA%`; si no, `home/.cache`.
pub fn cache_dir() -> Option<PathBuf> {
    if let Some(xdg) = nonempty("XDG_CACHE_HOME") {
        return Some(PathBuf::from(xdg));
    }
    #[cfg(windows)]
    if let Some(local) = nonempty("LOCALAPPDATA") {
        return Some(PathBuf::from(local));
    }
    Some(home_dir()?.join(".cache"))
}

/// `Command` para subprocesos en segundo plano. En Windows añade `CREATE_NO_WINDOW` para que
/// no parpadee ninguna consola.
pub fn cmd(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Abre `url` con el navegador por defecto. Sin `cmd /c start`, que rompe con `&`.
pub fn open_url(url: &str) {
    let mut command = if cfg!(windows) {
        let mut c = cmd("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = cmd("open");
        c.arg(url);
        c
    } else {
        let mut c = cmd("xdg-open");
        c.arg(url);
        c
    };
    let _ = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Muestra `path` en el gestor de ficheros: lo selecciona si es un fichero.
///
/// En Linux se intenta primero FileManager1 (Nautilus, Dolphin, Nemo… seleccionan el elemento)
/// y si falla, `xdg-open` con el directorio contenedor.
pub fn reveal(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut command = cmd("explorer");
        if path.is_dir() {
            command.arg(path);
        } else {
            // `explorer /select,"ruta"`: raw_arg conserva las comillas (tolera espacios).
            // explorer no entiende las '/' que llegan de git (`C:\repo/src/main.rs`).
            let path = path.to_string_lossy().replace('/', "\\");
            command.raw_arg(format!("/select,\"{path}\""));
        }
        let _ = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let uri = format!("file://{}", crate::app::uri_encode(&path.to_string_lossy()));
        let shown = cmd("dbus-send")
            .args([
                "--session",
                "--print-reply",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("array:string:{uri}"),
                "string:",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !shown {
            let dir = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
            let _ = cmd("xdg-open")
                .arg(dir)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

/// Mata el proceso `pid` y todos sus descendientes.
pub fn kill_tree(pid: u32) {
    let pid = pid.to_string();
    #[cfg(windows)]
    let mut command = {
        let mut c = cmd("taskkill");
        c.args(["/F", "/T", "/PID", pid.as_str()]);
        c
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut c = cmd("kill");
        c.args(["-9", pid.as_str()]);
        c
    };
    let _ = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Quita el prefijo `\\?\` que deja `canonicalize` en Windows (`\\?\UNC\srv\x` → `\\srv\x`).
pub fn strip_verbatim(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy().into_owned();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_verbatim_quita_prefijo() {
        let p = strip_verbatim(PathBuf::from(r"\\?\C:\Users\ana\Proyectos"));
        if cfg!(windows) {
            assert_eq!(p, PathBuf::from(r"C:\Users\ana\Proyectos"));
        } else {
            assert_eq!(p, PathBuf::from(r"\\?\C:\Users\ana\Proyectos"));
        }
    }

    #[cfg(windows)]
    #[test]
    fn strip_verbatim_unc() {
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\UNC\srv\share\a")),
            PathBuf::from(r"\\srv\share\a")
        );
    }

    #[test]
    fn home_config_y_cache_no_estan_vacios() {
        // En un entorno normal hay HOME (o USERPROFILE) y por tanto dirs derivados.
        if let Some(home) = home_dir() {
            assert!(config_dir().is_some_and(|c| c.starts_with(&home) || std::env::var_os("XDG_CONFIG_HOME").is_some()));
            assert!(cache_dir().is_some());
        }
    }
}
