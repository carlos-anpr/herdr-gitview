//! Explicaciones de cambios con IA (subproceso configurable + caché en disco). PROPIETARIO: OpenCode.

use crate::config::Config;
use crate::model::*;
use anyhow::{bail, Context, Result};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- caché en disco

/// `$XDG_CACHE_HOME/herdr-gitview/ai` o `~/.cache/herdr-gitview/ai`.
fn default_cache_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME")
        && !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("herdr-gitview").join("ai"));
        }
    let home = std::env::var_os("HOME")?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".cache").join("herdr-gitview").join("ai"))
}

/// FNV-1a de 64 bits.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn kind_tag(kind: AiKind) -> &'static str {
    match kind {
        AiKind::Commit => "commit",
        AiKind::File => "file",
        AiKind::MergeRequest => "mr",
    }
}

/// hex de 16 chars: FNV-1a 64 de `cache_key` + `kind`.
fn hash_key(req: &AiRequest) -> String {
    let raw = format!("{}|{}", req.cache_key, kind_tag(req.kind));
    format!("{:016x}", fnv1a64(raw.as_bytes()))
}

fn cache_file(cache_dir: &Path, req: &AiRequest) -> PathBuf {
    cache_dir.join(format!("{}.md", hash_key(req)))
}

fn cached_at(cache_dir: &Path, req: &AiRequest) -> Option<String> {
    std::fs::read_to_string(cache_file(cache_dir, req)).ok()
}

/// Escritura atómica: tmp en el mismo directorio + rename.
fn store_at(cache_dir: &Path, req: &AiRequest, text: &str) -> Result<()> {
    std::fs::create_dir_all(cache_dir)
        .with_context(|| format!("no se pudo crear {}", cache_dir.display()))?;
    let dest = cache_file(cache_dir, req);
    let tmp = cache_dir.join(format!(".{}.{}.tmp", hash_key(req), std::process::id()));
    std::fs::write(&tmp, text.as_bytes())
        .with_context(|| format!("no se pudo escribir {}", tmp.display()))?;
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("no se pudo guardar {}", dest.display()));
    }
    Ok(())
}

/// Respuesta cacheada si existe (~/.cache/herdr-gitview/ai/<hash(cache_key)>.md).
pub fn cached(req: &AiRequest) -> Option<String> {
    let dir = default_cache_dir()?;
    cached_at(&dir, req)
}

// ---------------------------------------------------------------- prompt

/// Trunca a `max` bytes en frontera de línea (si no hay salto en el límite, en frontera de
/// carácter). Devuelve (texto, bytes omitidos).
fn truncate_diff(diff: &str, max: usize) -> (String, usize) {
    let total = diff.len();
    if total <= max {
        return (diff.to_string(), 0);
    }
    let mut cut = match diff.get(..max).and_then(|h| h.rfind('\n')) {
        Some(i) => i + 1, // conserva el salto final
        None => {
            let mut c = max.min(total);
            while c > 0 && !diff.is_char_boundary(c) {
                c -= 1;
            }
            c
        }
    };
    if cut == 0 {
        // el primer carácter es más ancho que `max`: al menos ese carácter completo
        cut = diff.chars().next().map(|c| c.len_utf8()).unwrap_or(0).min(total);
    }
    (diff[..cut].to_string(), total - cut)
}

/// Construye el prompt en español, corto y al grano, con las secciones del tipo de `req`.
pub fn build_prompt(cfg: &Config, req: &AiRequest) -> String {
    let (diff, omitidos) = truncate_diff(&req.diff, cfg.ai_max_diff_bytes);

    let mut p = String::new();
    p.push_str(
        "Eres un revisor de código senior. Responde en español, corto, al grano y útil: \
sin relleno, sin repetir lo que ya dijiste y sin tablas.\n",
    );
    p.push_str(&format!("\nTipo: {}\n", kind_tag(req.kind)));
    p.push_str(&format!("Título: {}\n", req.title));
    if !req.context.trim().is_empty() {
        p.push_str("\nContexto:\n");
        p.push_str(req.context.trim());
        p.push('\n');
    }
    p.push_str("\nDiff:\n```diff\n");
    p.push_str(&diff);
    p.push_str("\n```\n");
    if omitidos > 0 {
        p.push_str(&format!("\n[diff truncado: {omitidos} bytes omitidos]\n"));
    }

    p.push_str("\nFormato de respuesta:\n");
    match req.kind {
        AiKind::Commit => {
            p.push_str("- Primera línea: «Propósito:» y en ella, en UNA sola línea, el propósito del commit.\n");
            p.push_str("- «Qué cambia»: viñetas con lo esencial (máximo 6).\n");
            p.push_str("- «Riesgos»: solo si los hay, como máximo 2 viñetas.\n");
        }
        AiKind::File => {
            p.push_str("- Máximo 5 viñetas sobre qué ha cambiado en ESTE fichero y por qué importa.\n");
            p.push_str("- No repitas el diff ni cites bloques largos: habla de la intención del cambio.\n");
        }
        AiKind::MergeRequest => {
            p.push_str("- «Propósito»: 1-2 líneas.\n");
            p.push_str("- «Cambios clave»: viñetas (máximo 6).\n");
            p.push_str("- «Por fichero»: una línea por fichero relevante, con el formato `ruta — qué cambia`.\n");
            p.push_str("- «A vigilar»: solo si aplica (riesgos, tests tocados, despliegue).\n");
        }
    }
    p
}

// ---------------------------------------------------------------- subproceso

fn read_all<R: Read>(pipe: Option<R>) -> String {
    let Some(mut r) = pipe else { return String::new() };
    let mut bytes = Vec::new();
    if r.read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn join(h: thread::JoinHandle<String>) -> String {
    h.join().unwrap_or_default()
}

/// Últimas 5 líneas de stderr, listas para pegar a un error.
fn tail5(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().collect();
    let start = lines.len().saturating_sub(5);
    let chunk = &lines[start..];
    if chunk.is_empty() {
        return String::new();
    }
    format!("\n--- últimas 5 líneas de stderr ---\n{}\n", chunk.join("\n"))
}

/// Lanza `cfg.ai_command`, prompt por stdin, lee stdout/stderr en hilos (evita deadlock),
/// aplica timeout con `try_wait`. Devuelve stdout recortado.
fn run(cfg: &Config, prompt: &str) -> Result<String> {
    let argv = &cfg.ai_command;
    if argv.is_empty() {
        bail!("ai_command está vacío");
    }
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("no se pudo lanzar «{}»", argv[0]))?;

    // Lectores antes de escribir: un hijo que escriba mucho antes de leer stdin no bloquea.
    let out_pipe = child.stdout.take();
    let err_pipe = child.stderr.take();
    let h_out = thread::spawn(move || read_all(out_pipe));
    let h_err = thread::spawn(move || read_all(err_pipe));
    {
        let mut stdin = child.stdin.take().context("sin stdin en el subproceso")?;
        // Un hijo que no lee stdin (EPIPE) no es error aquí: lo decide su código de salida.
        let _ = stdin.write_all(prompt.as_bytes());
    }

    let deadline = Instant::now() + Duration::from_secs(cfg.ai_timeout_secs);
    let status = loop {
        if let Some(st) = child.try_wait().context("error al consultar el subproceso")? {
            break st;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            // No se unen los hilos de lectura: si un nieto hereda el pipe quedan colgados y
            // bloquearían aquí. Al soltarlos se desenganchan solos cuando el pipe se cierre.
            bail!(
                "timeout: «{}» no respondió en {} s",
                argv[0],
                cfg.ai_timeout_secs
            );
        }
        thread::sleep(Duration::from_millis(20));
    };

    let stdout = join(h_out);
    let stderr = join(h_err);
    check(status, argv.first().map(String::as_str).unwrap_or("?"), &stdout, &stderr)
}

fn check(status: ExitStatus, cmd: &str, stdout: &str, stderr: &str) -> Result<String> {
    if !status.success() {
        let code = status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "señal".to_string());
        bail!("{cmd} terminó con código {code}{}", tail5(stderr));
    }
    let text = stdout.trim();
    if text.is_empty() {
        bail!("salida vacía de «{cmd}»{}", tail5(stderr));
    }
    Ok(text.to_string())
}

/// El comando emite el stream-json de Claude Code (`--output-format stream-json`).
fn is_stream(cfg: &Config) -> bool {
    cfg.ai_command.iter().any(|a| a == "stream-json")
}

/// Lanza `argv`, le pasa `input` por stdin y entrega cada línea de stdout a `on_line` según llega.
/// Aplica el timeout matando al hijo. Devuelve el estado de salida y el stderr.
fn stream_lines(argv: &[String], input: Vec<u8>, timeout_secs: u64, on_line: &mut dyn FnMut(&str)) -> Result<(ExitStatus, String)> {
    if argv.is_empty() {
        bail!("comando de IA vacío");
    }
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("no se pudo lanzar «{}»", argv[0]))?;
    let pid = child.id();
    let err_pipe = child.stderr.take();
    let h_err = thread::spawn(move || read_all(err_pipe));
    let mut stdin = child.stdin.take().context("sin stdin en el subproceso")?;
    let h_in = thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });

    // Vigilante de timeout: mata al hijo si no termina a tiempo.
    let done = Arc::new(AtomicBool::new(false));
    let timed_out = Arc::new(AtomicBool::new(false));
    {
        let (done, timed_out) = (done.clone(), timed_out.clone());
        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                if Instant::now() >= deadline {
                    timed_out.store(true, Ordering::Relaxed);
                    let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
    }
    let out = child.stdout.take().context("sin stdout en el subproceso")?;
    for line in BufReader::new(out).lines() {
        let Ok(line) = line else { break };
        on_line(&line);
    }
    done.store(true, Ordering::Relaxed);
    let status = child.wait().context("error al esperar al subproceso")?;
    let _ = h_in.join();
    let stderr = join(h_err);
    if timed_out.load(Ordering::Relaxed) {
        bail!("timeout: «{}» no respondió en {timeout_secs} s", argv[0]);
    }
    Ok((status, stderr))
}

/// Como `run`, pero leyendo el stream-json de Claude Code y avisando con el texto acumulado.
fn run_stream(cfg: &Config, prompt: &str, on_text: &mut dyn FnMut(&str)) -> Result<String> {
    let mut text = String::new();
    let mut result: Option<String> = None;
    let mut is_error = false;
    let (status, stderr) = stream_lines(&cfg.ai_command, prompt.as_bytes().to_vec(), cfg.ai_timeout_secs, &mut |line| {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { return };
        match v["type"].as_str() {
            Some("stream_event") => {
                let ev = &v["event"];
                if ev["type"] == "content_block_delta"
                    && ev["delta"]["type"] == "text_delta"
                    && let Some(t) = ev["delta"]["text"].as_str()
                {
                    text.push_str(t);
                    on_text(&text);
                }
            }
            Some("result") => {
                result = v["result"].as_str().map(str::to_owned);
                is_error = v["is_error"].as_bool().unwrap_or(false);
            }
            _ => {}
        }
    })?;
    if is_error {
        bail!("{}{}", result.unwrap_or_default(), tail5(&stderr));
    }
    let out = result.filter(|r| !r.trim().is_empty()).unwrap_or(text);
    check(status, &cfg.ai_command[0], &out, &stderr)
}

const SYSTEM: &str = "Eres un revisor de código senior. Respondes en español, breve y al grano, en Markdown simple sin tablas.";

/// Clave del endpoint: variable de entorno o, si se indica `ai_api_auth_entry`, esa entrada del
/// auth.json de OpenCode. Sin ninguna de las dos, `None` (endpoints locales sin clave).
fn api_key(cfg: &Config) -> Result<Option<String>> {
    if !cfg.ai_api_key_env.is_empty()
        && let Ok(k) = std::env::var(&cfg.ai_api_key_env)
        && !k.trim().is_empty()
    {
        return Ok(Some(k.trim().to_owned()));
    }
    if cfg.ai_api_auth_entry.is_empty() {
        return Ok(None);
    }
    let home = std::env::var_os("HOME").context("sin HOME")?;
    let path = PathBuf::from(home).join(".local/share/opencode/auth.json");
    let raw = std::fs::read_to_string(&path).with_context(|| format!("sin clave: define {} o inicia sesión en OpenCode", cfg.ai_api_key_env))?;
    let v: serde_json::Value = serde_json::from_str(&raw).context("auth.json de OpenCode ilegible")?;
    v[&cfg.ai_api_auth_entry]["key"]
        .as_str()
        .map(|k| Some(k.to_owned()))
        .with_context(|| format!("auth.json no tiene clave para «{}»", cfg.ai_api_auth_entry))
}

/// Cuerpo JSON de chat/completions con streaming, mezclando `ai_api_extra`.
fn api_body(cfg: &Config, prompt: &str) -> Result<Vec<u8>> {
    let mut body = serde_json::json!({
        "model": cfg.ai_api_model,
        "stream": true,
        "messages": [
            {"role": "system", "content": SYSTEM},
            {"role": "user", "content": prompt},
        ],
    });
    if !cfg.ai_api_extra.trim().is_empty() {
        let extra: serde_json::Value = serde_json::from_str(&cfg.ai_api_extra).context("ai_api_extra no es JSON válido")?;
        if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                b.insert(k.clone(), v.clone());
            }
        }
    }
    Ok(serde_json::to_vec(&body)?)
}

/// Extrae el texto de una línea SSE (`data: {...}`) de chat/completions.
fn sse_delta(line: &str) -> Option<String> {
    let data = line.strip_prefix("data:")?.trim();
    if data == "[DONE]" {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(data).ok()?;
    v["choices"][0]["delta"]["content"].as_str().map(str::to_owned)
}

/// Endpoint OpenAI-compatible vía `curl` (la clave va en un fichero de config 0600, no en argv).
fn run_api(cfg: &Config, prompt: &str, on_text: &mut dyn FnMut(&str)) -> Result<String> {
    let key = api_key(cfg)?;
    let dir = default_cache_dir().context("sin directorio de caché")?;
    std::fs::create_dir_all(&dir)?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let conf = dir.join(format!(".curl-{}-{nanos}", std::process::id()));
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&conf)?;
        if let Some(key) = &key {
            writeln!(f, "header = \"Authorization: Bearer {key}\"")?;
        }
        if cfg.ai_api_url.contains("opencode.ai") {
            writeln!(f, "header = \"x-opencode-session: gitview-{nanos}\"")?;
        }
        writeln!(f, "header = \"Content-Type: application/json\"")?;
    }
    let argv: Vec<String> = ["curl", "-sS", "-N", "--fail-with-body", "-K"]
        .iter()
        .map(|s| s.to_string())
        .chain([conf.to_string_lossy().into_owned(), "--data-binary".into(), "@-".into(), cfg.ai_api_url.clone()])
        .collect();
    let mut text = String::new();
    let mut raw = String::new();
    let res = api_body(cfg, prompt).and_then(|body| {
        stream_lines(&argv, body, cfg.ai_timeout_secs, &mut |line| {
            if let Some(t) = sse_delta(line) {
                text.push_str(&t);
                on_text(&text);
            } else if !line.starts_with("data:") && raw.len() < 2000 {
                raw.push_str(line);
                raw.push('\n');
            }
        })
    });
    let _ = std::fs::remove_file(&conf);
    let (status, stderr) = res?;
    if !status.success() || text.trim().is_empty() {
        let detail = if raw.trim().is_empty() { stderr.trim().to_owned() } else { raw.trim().to_owned() };
        bail!("{} ({}): {}", cfg.ai_api_model, cfg.ai_api_url, detail);
    }
    Ok(text.trim().to_owned())
}

// ---------------------------------------------------------------- API pública

/// Igual que `explain`, con `force` para ignorar la caché y `on_text` para recibir el texto
/// parcial mientras llega (solo con comandos stream-json; si no, se llama al final).
pub fn explain_live(cfg: &Config, req: &AiRequest, force: bool, on_text: &mut dyn FnMut(&str)) -> Result<String> {
    let dir = default_cache_dir().context("sin HOME ni XDG_CACHE_HOME: no hay caché")?;
    if !force && let Some(hit) = cached_at(&dir, req) {
        return Ok(hit);
    }
    let prompt = build_prompt(cfg, req);
    if cfg.ai_backend == "api" && (cfg.ai_api_url.trim().is_empty() || cfg.ai_api_model.trim().is_empty()) {
        bail!("IA sin configurar: define ai_api_url y ai_api_model en ~/.config/herdr-gitview/config.toml");
    }
    if cfg.ai_backend != "api" && cfg.ai_command.is_empty() {
        bail!("IA sin configurar: define ai_command en ~/.config/herdr-gitview/config.toml");
    }
    let text = if cfg.ai_backend == "api" {
        run_api(cfg, &prompt, on_text)?
    } else if is_stream(cfg) {
        run_stream(cfg, &prompt, on_text)?
    } else {
        run(cfg, &prompt)?
    };
    let _ = store_at(&dir, req, &text);
    Ok(text)
}

#[cfg_attr(not(test), allow(dead_code))]
fn explain_at(cfg: &Config, req: &AiRequest, cache_dir: &Path) -> Result<String> {
    if let Some(hit) = cached_at(cache_dir, req) {
        return Ok(hit);
    }
    let prompt = build_prompt(cfg, req);
    let text = run(cfg, &prompt)?;
    let _ = store_at(cache_dir, req, &text); // la caché es best-effort
    Ok(text)
}

/// Construye el prompt, lanza `cfg.ai_command` con el prompt por stdin, aplica timeout,
/// guarda en caché y devuelve el texto. Bloqueante.
#[allow(dead_code)] // API del contrato; la UI usa explain_live
pub fn explain(cfg: &Config, req: &AiRequest) -> Result<String> {
    let dir = default_cache_dir().context("sin HOME ni XDG_CACHE_HOME: no hay caché")?;
    explain_at(cfg, req, &dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_extrae_contenido() {
        assert_eq!(sse_delta(r#"data: {"choices":[{"delta":{"content":"Ho"}}]}"#).as_deref(), Some("Ho"));
        assert_eq!(sse_delta(r#"data: {"choices":[{"delta":{"content":null,"reasoning_content":"x"}}]}"#), None);
        assert_eq!(sse_delta("data: [DONE]"), None);
        assert_eq!(sse_delta(": keepalive"), None);
    }

    #[test]
    fn cuerpo_api_mezcla_extra() {
        let cfg = Config {
            ai_api_model: "modelo-x".into(),
            ai_api_extra: r#"{"thinking":{"type":"disabled"}}"#.into(),
            ..Config::default()
        };
        let v: serde_json::Value = serde_json::from_slice(&api_body(&cfg, "hola").unwrap()).unwrap();
        assert_eq!(v["model"], "modelo-x");
        assert_eq!(v["thinking"]["type"], "disabled");
        assert_eq!(v["messages"][1]["content"], "hola");
    }

    #[test]
    fn stream_json_acumula_y_usa_result() {
        let script = r#"cat >/dev/null
echo '{"type":"system"}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Ho"}}}'
echo 'no json'
echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"la"}}}'
echo '{"type":"result","is_error":false,"result":"Hola"}'"#;
        let cfg = Config {
            ai_command: vec!["sh".into(), "-c".into(), script.into(), "stream-json".into()],
            ai_timeout_secs: 5,
            ..Config::default()
        };
        let mut parts = Vec::new();
        let out = run_stream(&cfg, "x", &mut |t| parts.push(t.to_string())).unwrap();
        assert_eq!(out, "Hola");
        assert_eq!(parts, ["Ho", "Hola"]);
    }

    /// Dir de caché temporal para tests (se borra al final de cada test).
    fn temp_cache_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "herdr-gitview-ai-{}-{tag}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn req(kind: AiKind, key: &str) -> AiRequest {
        AiRequest {
            cache_key: key.to_string(),
            kind,
            title: "arregla el cálculo de IVA".to_string(),
            context: "autor: Ana\nrama: fix/iva".to_string(),
            diff: "diff de prueba\n".to_string(),
        }
    }

    #[test]
    fn hash_estable() {
        let dir = Path::new("/tmp");
        let a = cache_file(dir, &req(AiKind::Commit, "commit:abc"));
        let b = cache_file(dir, &req(AiKind::Commit, "commit:abc"));
        assert_eq!(a, b);
        let name = a.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.ends_with(".md"));
        let hex = name.trim_end_matches(".md");
        assert_eq!(hex.len(), 16);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
        // distinta clave o distinto kind → distinto fichero
        assert_ne!(a, cache_file(dir, &req(AiKind::Commit, "commit:otro")));
        assert_ne!(a, cache_file(dir, &req(AiKind::File, "commit:abc")));
    }

    #[test]
    fn hash_fnv_conocido() {
        // vector de FNV-1a 64: "a" → 0xaf63dc4c8601ec8c
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn truncado_en_frontera_de_linea() {
        let diff = "linea uno\nlinea dos\nlinea tres\n";
        let (t, omit) = truncate_diff(diff, 15);
        assert!(t.len() <= 15);
        assert!(t.ends_with('\n'), "cortamos en frontera de línea: {t:?}");
        assert_eq!(t.len() + omit, diff.len());
        assert!(diff.get(t.len()..).is_some(), "el resto es un sufijo válido");

        // sin salto dentro del límite → se corta igual, en carácter válido
        let (t2, omit2) = truncate_diff("abcdefghij", 4);
        assert_eq!(t2, "abcd");
        assert_eq!(omit2, 6);

        // por debajo del límite no se toca
        let (t3, omit3) = truncate_diff("hola", 100);
        assert_eq!(t3, "hola");
        assert_eq!(omit3, 0);

        // UTF-8: nunca corta a mitad de un carácter
        let largo: String = std::iter::repeat("áéíóúñ ").take(50).collect();
        let (t4, omit4) = truncate_diff(&largo, 20);
        assert!(largo.get(..t4.len()).is_some());
        assert_eq!(t4.len() + omit4, largo.len());
    }

    #[test]
    fn prompt_con_secciones_por_tipo() {
        let cfg = Config::default();

        let mut c = req(AiKind::Commit, "commit:x");
        c.diff = "@@ -1 +1 @@\n-a\n+b\n".to_string();
        let p = build_prompt(&cfg, &c);
        assert!(p.contains("Propósito"));
        assert!(p.contains("Qué cambia"));
        assert!(p.contains("Riesgos"));
        assert!(p.contains(&c.title));
        assert!(p.contains(&c.context));
        assert!(!p.contains("Por fichero"));
        assert!(!p.contains("A vigilar"));

        let f = req(AiKind::File, "file:x");
        let p = build_prompt(&cfg, &f);
        assert!(p.contains("Máximo 5 viñetas"));
        assert!(p.contains("ESTE fichero"));
        assert!(p.contains("Título:"));
        assert!(!p.contains("Qué cambia"));

        let mut m = req(AiKind::MergeRequest, "mr:x");
        m.context = "rama fix/iva → main".to_string();
        let p = build_prompt(&cfg, &m);
        assert!(p.contains("Propósito"));
        assert!(p.contains("Cambios clave"));
        assert!(p.contains("Por fichero"));
        assert!(p.contains("A vigilar"));
        assert!(p.contains(&m.context));
    }

    #[test]
    fn prompt_trunca_diff_y_anota_bytes() {
        let mut cfg = Config::default();
        let linea = "linea de diff suficientemente larga\n"; // 35 bytes
        cfg.ai_max_diff_bytes = linea.len();
        let mut r = req(AiKind::Commit, "commit:big");
        r.diff = std::iter::repeat(linea).take(20).collect();
        let p = build_prompt(&cfg, &r);
        let dentro = p
            .split("```diff\n")
            .nth(1)
            .expect("bloque de diff")
            .split("\n```")
            .next()
            .unwrap();
        assert!(dentro.len() < r.diff.len(), "el diff se truncó");
        assert!(dentro.ends_with('\n'), "corte en frontera de línea");
        assert_eq!(dentro, &r.diff[..dentro.len()]);
        let nota = format!("[diff truncado: {} bytes omitidos]", r.diff.len() - dentro.len());
        assert!(p.contains(&nota), "falta la nota: {p}");
    }

    #[test]
    fn explain_ok_y_cache() {
        let dir = temp_cache_dir("ok");
        let mut cfg = Config::default();
        cfg.ai_command = vec![
            "sh".into(),
            "-c".into(),
            "cat >/dev/null; echo hola".into(),
        ];
        let r = req(AiKind::Commit, "commit:ok");
        let out = explain_at(&cfg, &r, &dir).expect("explain debe funcionar");
        assert_eq!(out, "hola");
        assert!(cache_file(&dir, &r).exists(), "guarda en caché");
        // segunda llamada: sale de caché aunque el comando ya no sirva
        let mut roto = cfg.clone();
        roto.ai_command = vec!["sh".into(), "-c".into(), "exit 7".into()];
        assert_eq!(explain_at(&roto, &r, &dir).unwrap(), "hola");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn explain_falla_por_timeout() {
        let dir = temp_cache_dir("timeout");
        let mut cfg = Config::default();
        cfg.ai_command = vec!["sh".into(), "-c".into(), "sleep 5".into()];
        cfg.ai_timeout_secs = 1;
        let r = req(AiKind::Commit, "commit:timeout");
        let t0 = Instant::now();
        let err = explain_at(&cfg, &r, &dir).expect_err("debe fallar por timeout");
        let msg = format!("{err:#}");
        assert!(msg.contains("timeout"), "mensaje: {msg}");
        assert!(t0.elapsed() < Duration::from_secs(4), "mató el proceso a tiempo");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn explain_falla_con_rc_o_salida_vacia() {
        let dir = temp_cache_dir("rc");
        let mut cfg = Config::default();
        cfg.ai_command = vec![
            "sh".into(),
            "-c".into(),
            "cat >/dev/null; echo boom >&2; exit 3".into(),
        ];
        let r = req(AiKind::Commit, "commit:rc");
        let err = format!("{:#}", explain_at(&cfg, &r, &dir).expect_err("rc≠0"));
        assert!(err.contains('3'), "mensaje: {err}");
        assert!(err.contains("boom"), "últimas líneas de stderr: {err}");

        cfg.ai_command = vec!["sh".into(), "-c".into(), "cat >/dev/null".into()];
        let err = format!("{:#}", explain_at(&cfg, &r, &dir).expect_err("vacía"));
        assert!(err.contains("vacía"), "mensaje: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
