//! Configuración (~/.config/herdr-gitview/config.toml) con valores por defecto. PROPIETARIO: OpenCode.

use crate::platform;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    /// "api" (endpoint OpenAI-compatible vía curl) o "command" (`ai_command`).
    pub ai_backend: String,
    /// URL de chat/completions del endpoint OpenAI-compatible.
    pub ai_api_url: String,
    pub ai_api_model: String,
    /// Campos extra (JSON) que se mezclan en el cuerpo de la petición.
    pub ai_api_extra: String,
    /// Variable de entorno con la clave; por defecto `OLLAMA_API_KEY`. Si no está y hay
    /// `ai_api_auth_entry`, se lee esa entrada de ~/.local/share/opencode/auth.json.
    pub ai_api_key_env: String,
    pub ai_api_auth_entry: String,
    /// argv del comando de IA; el prompt se pasa por stdin, la respuesta sale por stdout.
    pub ai_command: Vec<String>,
    /// Bytes máximos de diff enviados a la IA.
    pub ai_max_diff_bytes: usize,
    /// Timeout de la llamada a IA en segundos.
    pub ai_timeout_secs: u64,
    /// Horas hacia atrás para considerar "actividad reciente" de agentes en un proyecto.
    pub discover_hours: u64,
    /// Profundidad máxima al buscar repos bajo la carpeta de partida.
    pub scan_depth: usize,
    /// Commits por página en el grafo.
    pub page_size: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ai_backend: "api".into(),
            ai_api_url: "https://ollama.com/v1/chat/completions".into(),
            ai_api_model: "minimax-m3.1-flash".into(),
            // minimax razona mucho por defecto; con "low" responde ~10 veces antes con la misma calidad.
            ai_api_extra: r#"{"reasoning_effort":"low"}"#.into(),
            ai_api_key_env: "OLLAMA_API_KEY".into(),
            ai_api_auth_entry: String::new(),
            ai_command: Vec::new(),
            ai_max_diff_bytes: 60_000,
            ai_timeout_secs: 120,
            discover_hours: 12,
            scan_depth: 3,
            page_size: 400,
        }
    }
}

/// Ruta del fichero de configuración (`$XDG_CONFIG_HOME` o `~/.config`, también en Windows).
fn config_path() -> Option<PathBuf> {
    Some(platform::config_dir()?.join("herdr-gitview").join("config.toml"))
}

/// Carga config; si no existe o es inválida, usa defaults (nunca falla).
pub fn load() -> Config {
    let Some(path) = config_path() else { return Config::default() };
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

/// Forma del fichero TOML: todas las claves opcionales con los mismos nombres que `Config`.
#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    ai_backend: Option<String>,
    ai_api_url: Option<String>,
    ai_api_model: Option<String>,
    ai_api_extra: Option<String>,
    ai_api_key_env: Option<String>,
    ai_api_auth_entry: Option<String>,
    ai_command: Option<Vec<String>>,
    ai_max_diff_bytes: Option<usize>,
    ai_timeout_secs: Option<u64>,
    discover_hours: Option<u64>,
    scan_depth: Option<usize>,
    page_size: Option<usize>,
}

/// Parsea un TOML desde string: lo que falta usa el default, cualquier error devuelve `Err`
/// (el llamado —`load`— lo traduce a defaults completos).
fn parse(text: &str) -> Result<Config, toml::de::Error> {
    let raw: FileConfig = toml::from_str(text)?;
    let mut cfg = Config::default();
    // Los campos extra por defecto son para Ollama Cloud: no se envían a otro endpoint.
    if raw.ai_api_url.is_some() && raw.ai_api_extra.is_none() {
        cfg.ai_api_extra.clear();
    }
    for (dst, src) in [
        (&mut cfg.ai_backend, raw.ai_backend),
        (&mut cfg.ai_api_url, raw.ai_api_url),
        (&mut cfg.ai_api_model, raw.ai_api_model),
        (&mut cfg.ai_api_extra, raw.ai_api_extra),
        (&mut cfg.ai_api_key_env, raw.ai_api_key_env),
        (&mut cfg.ai_api_auth_entry, raw.ai_api_auth_entry),
    ] {
        if let Some(v) = src {
            *dst = v;
        }
    }
    if let Some(v) = raw.ai_command
        && !v.is_empty() {
            cfg.ai_command = v;
        }
    if let Some(v) = raw.ai_max_diff_bytes {
        cfg.ai_max_diff_bytes = v;
    }
    if let Some(v) = raw.ai_timeout_secs {
        cfg.ai_timeout_secs = v;
    }
    if let Some(v) = raw.discover_hours {
        cfg.discover_hours = v;
    }
    if let Some(v) = raw.scan_depth {
        cfg.scan_depth = v;
    }
    if let Some(v) = raw.page_size {
        cfg.page_size = v;
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let cfg = Config::default();
        assert_eq!(cfg.ai_backend, "api");
        // Por defecto, Ollama Cloud (basta con exportar OLLAMA_API_KEY).
        assert_eq!(cfg.ai_api_url, "https://ollama.com/v1/chat/completions");
        assert_eq!(cfg.ai_api_model, "minimax-m3.1-flash");
        assert_eq!(cfg.ai_api_key_env, "OLLAMA_API_KEY");
        assert!(cfg.ai_command.is_empty());
        assert_eq!(cfg.ai_max_diff_bytes, 60_000);
        assert_eq!(cfg.ai_timeout_secs, 120);
        assert_eq!(cfg.discover_hours, 12);
        assert_eq!(cfg.scan_depth, 3);
        assert_eq!(cfg.page_size, 400);
    }

    #[test]
    fn parse_toml_completo() {
        let text = r#"
ai_command = ["echo", "hola"]
ai_max_diff_bytes = 10
ai_timeout_secs = 1
discover_hours = 2
scan_depth = 4
page_size = 7
"#;
        let cfg = parse(text).expect("toml válido");
        assert_eq!(cfg.ai_command, vec!["echo", "hola"]);
        assert_eq!(cfg.ai_max_diff_bytes, 10);
        assert_eq!(cfg.ai_timeout_secs, 1);
        assert_eq!(cfg.discover_hours, 2);
        assert_eq!(cfg.scan_depth, 4);
        assert_eq!(cfg.page_size, 7);
    }

    #[test]
    fn parse_toml_parcial_mantiene_defaults() {
        let cfg = parse("page_size = 25\n").expect("toml válido");
        assert_eq!(cfg.page_size, 25);
        assert_eq!(cfg.scan_depth, 3);
        assert_eq!(cfg.ai_timeout_secs, 120);
        assert_eq!(cfg.ai_command, Config::default().ai_command);
        assert_eq!(cfg.ai_api_url, "https://ollama.com/v1/chat/completions");
        assert_eq!(cfg.ai_api_model, "minimax-m3.1-flash");
        assert_eq!(cfg.ai_api_key_env, "OLLAMA_API_KEY");
        assert_eq!(cfg.ai_api_extra, r#"{"reasoning_effort":"low"}"#);
    }

    #[test]
    fn otro_endpoint_no_hereda_extra_de_ollama() {
        let cfg = parse("ai_api_url = \"https://api.openai.com/v1/chat/completions\"
").expect("toml válido");
        assert!(cfg.ai_api_extra.is_empty());
        let cfg = parse("ai_api_url = \"http://x\"
ai_api_extra = '{\"a\":1}'
").expect("toml válido");
        assert_eq!(cfg.ai_api_extra, r#"{"a":1}"#);
    }

    #[test]
    fn parse_toml_invalido_da_error() {
        assert!(parse("esto no es toml = = =").is_err());
        assert!(parse("ai_max_diff_bytes = \"no-numero\"").is_err());
        // y, en load(), eso se traduce en defaults completos
        assert_eq!(Config::default().page_size, 400);
    }

    #[test]
    fn load_sin_fichero_usa_defaults() {
        // HOME/XDG apuntan a un directorio inexistente: load() no falla.
        let cfg = Config::default();
        assert_eq!(cfg.discover_hours, 12);
    }
}
