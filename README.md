<div align="center">

# 🌿 herdr-gitview

### *El control de cambios de tus agentes, sin salir de la terminal*

[![Rust](https://img.shields.io/badge/Rust-2024-dea584?style=flat-square&logo=rust&logoColor=black)](https://www.rust-lang.org)
[![ratatui](https://img.shields.io/badge/ratatui-TUI-7c3aed?style=flat-square)](https://ratatui.rs)
[![herdr](https://img.shields.io/badge/herdr-plugin-0ea5e9?style=flat-square)](https://herdr.dev)
[![GitLab](https://img.shields.io/badge/GitLab-Merge_Requests-fc6d26?style=flat-square&logo=gitlab&logoColor=white)](https://gitlab.com/gitlab-org/cli)
[![LLM](https://img.shields.io/badge/IA-cualquier_proveedor-412991?style=flat-square)](#-explicaciones-con-ia)
[![License](https://img.shields.io/badge/License-MIT-339933?style=flat-square)](LICENSE)

**Grafo de commits al estilo Git Graph de VS Code, diffs, Merge Requests de GitLab y explicaciones breves con IA.
Un plugin de [herdr](https://herdr.dev) que se abre sobre el panel donde trabaja tu agente, y también funciona solo.**

</div>

---

## ✨ Qué hace

Cuando tienes varios agentes (Claude Code, Codex, OpenCode…) tocando repos a la vez, la pregunta es siempre la misma:
*¿qué ha cambiado aquí?* herdr-gitview te lo enseña en una ventana flotante con un atajo de teclado.

| Vista | Qué muestra |
|---|---|
| **1 · Grafo** | Todas las ramas con carriles de colores, ramas y tags, cambios sin commit, ficheros del commit y su diff |
| **2 · Merge Requests** | MRs abiertos, fusionados, cerrados o todos, con ramas, pipeline, commits, ficheros y diff |
| **3 · Ficheros** | Explorador con el estado git de cada fichero y su historial |

- 🎯 **Abre el repo adecuado**: si el panel está en un repo, abre ese. Si está en una carpeta con muchos repos,
  propone los que un agente ha tocado hace poco: paneles de herdr, sesiones recientes de Claude Code y Codex,
  y actividad de git.
- 🤖 **Explicación con IA en una tecla**: `e` resume un commit, un fichero o un MR. La respuesta aparece mientras
  se genera y se guarda en caché.
- 🖱️ **Teclado y ratón**: clic, doble clic, rueda y pestañas y botones clicables.
- ⚡ **Rápido**: el grafo se pagina y todo lo lento (git, GitLab, IA) va en segundo plano.

## 📦 Requisitos

- [herdr](https://herdr.dev) ≥ 0.8 (opcional: también se usa como TUI independiente).
- Rust (edición 2024) y `git`.
- Para la vista de MRs: [`glab`](https://gitlab.com/gitlab-org/cli) autenticado contra el GitLab del remoto `origin`
  (gitlab.com o una instancia propia). Si el remoto no es GitLab, la pestaña no aparece.
- Para la IA: `curl` y una clave de cualquier API compatible con OpenAI, o una CLI de agente (ver abajo).

## 🚀 Instalación

```sh
git clone https://github.com/carlos-anpr/herdr-gitview.git
cd herdr-gitview
cargo build --release
herdr plugin link .
```

Asigna un atajo en `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+alt+v"
type = "plugin_action"
command = "local.gitview.open"
```

## ⌨️ Uso

En herdr, **prefijo + Alt+V** abre el visor sobre el panel activo.

| Tecla | Acción |
|---|---|
| `1` `2` `3` | Grafo · Merge Requests · Ficheros |
| `↑` `↓` `⏎` `esc` | Mover, abrir, volver |
| `[` `]` | Fichero anterior / siguiente con el diff abierto |
| `n` `N` | Siguiente / anterior hunk |
| `e` / `E` | Explicar con IA / regenerar la explicación |
| `J` `K` · `z` | Desplazar la explicación · ampliarla |
| `a` · `o` | MRs: cambiar filtro de estado · abrir en el navegador |
| `y` | Copiar el SHA (o la URL del MR) |
| `r` · `p` · `q` | Recargar · selector de proyectos · salir |
| `?` | Ayuda |

Fuera de herdr:

```sh
herdr-gitview [RUTA] [--pick]   # --pick fuerza el selector de proyectos
herdr-gitview --discover        # diagnóstico: qué repos detecta y por qué
```

## 🤖 Explicaciones con IA

No viene atado a ningún proveedor ni trae ninguno por defecto: sin configurar, `e` te recuerda qué falta.
Hay dos formas de conectarlo en `~/.config/herdr-gitview/config.toml`.

**1 · Cualquier API compatible con OpenAI** (`ai_backend = "api"`, respuesta en streaming):

```toml
ai_backend = "api"
ai_api_url = "https://api.groq.com/openai/v1/chat/completions"
ai_api_model = "qwen/qwen3.8-27b"
ai_api_key_env = "GROQ_API_KEY"        # variable de entorno con la clave
ai_api_extra = '{"reasoning_effort":"none"}'   # campos extra del cuerpo (JSON, opcional)
```

<details>
<summary><b>Otros endpoints</b></summary>

| Proveedor | `ai_api_url` |
|---|---|
| OpenAI | `https://api.openai.com/v1/chat/completions` |
| Groq | `https://api.groq.com/openai/v1/chat/completions` |
| xAI (Grok) | `https://api.x.ai/v1/chat/completions` |
| DeepSeek | `https://api.deepseek.com/v1/chat/completions` |
| OpenRouter | `https://openrouter.ai/api/v1/chat/completions` |
| OpenCode Go | `https://opencode.ai/inference/go/v1/chat/completions` |
| Ollama (local, sin clave) | `http://localhost:11434/v1/chat/completions` |

Si usas OpenCode, en vez de `ai_api_key_env` puedes poner `ai_api_auth_entry = "opencode-go"` y se reutiliza la clave
de `~/.local/share/opencode/auth.json`. La clave nunca va en la línea de comandos: se pasa a `curl` en un fichero
temporal con permisos `0600`.

</details>

**2 · Cualquier CLI** (`ai_backend = "command"`): recibe el prompt por stdin y devuelve la respuesta por stdout.

```toml
ai_backend = "command"
ai_command = ["codex", "exec", "-"]
# ai_command = ["opencode", "run"]
# Claude Code con streaming (la respuesta se pinta mientras llega):
# ai_command = ["claude", "-p", "--model", "haiku", "--tools", "", "--output-format", "stream-json",
#               "--include-partial-messages", "--verbose"]
```

Si el comando lleva `stream-json` (formato de Claude Code), el texto aparece según llega. Las respuestas se guardan
en `~/.cache/herdr-gitview/ai/`.

## ⚙️ Configuración

Todo es opcional:

```toml
ai_max_diff_bytes = 60000   # diff máximo que se envía a la IA
ai_timeout_secs = 120
discover_hours = 12         # ventana de "actividad reciente" de los agentes
scan_depth = 3              # profundidad al buscar repos bajo la carpeta del panel
page_size = 400             # commits por página en el grafo
```

## 📄 Licencia

[MIT](LICENSE)
