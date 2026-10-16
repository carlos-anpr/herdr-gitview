//! Tipos compartidos entre módulos. CONTRATO: no cambiar sin avisar al líder.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefKind {
    /// HEAD apuntando a una rama (se muestra junto a esa rama) o detached.
    Head,
    LocalBranch,
    RemoteBranch,
    Tag,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefKind,
}

#[derive(Debug, Clone, Default)]
pub struct Commit {
    /// SHA completo.
    pub id: String,
    /// SHA corto (7-10 chars).
    pub short: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    /// Fecha de autor, unix seconds.
    pub time: i64,
    pub subject: String,
    pub refs: Vec<RefLabel>,
}

#[derive(Debug, Clone, Default)]
pub struct CommitDetail {
    pub commit: Commit,
    /// Mensaje completo (subject + body).
    pub message: String,
    pub committer: String,
    pub commit_time: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    /// Ruta anterior si es rename/copy.
    pub old_path: Option<String>,
    /// 'A' añadido, 'M' modificado, 'D' borrado, 'R' renombrado, 'C' copiado, 'T' tipo.
    pub status: char,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
}

/// Entrada de `git status` (working tree + index).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: String,
    pub old_path: Option<String>,
    /// Columna X de porcelain v1 (index), ' ' si nada.
    pub index: char,
    /// Columna Y de porcelain v1 (worktree), ' ' si nada. '?' = untracked.
    pub worktree: char,
}

/// Celda del grafo: un carácter con el índice de color de su carril.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphCell {
    pub ch: char,
    /// Índice de color (el UI lo mapea a una paleta de ~8 colores con `color % N`).
    pub color: usize,
}

/// Una fila del grafo por commit (misma longitud y orden que el slice de commits).
#[derive(Debug, Clone, Default)]
pub struct GraphRow {
    /// Celdas a dibujar a la izquierda del mensaje. Ancho = 2 * nº de carriles activos en la fila.
    pub cells: Vec<GraphCell>,
    /// Carril donde está el nodo del commit.
    pub lane: usize,
    /// Color del nodo del commit.
    pub color: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MrState {
    Opened,
    Merged,
    Closed,
    All,
}

#[derive(Debug, Clone, Default)]
pub struct MergeRequest {
    pub iid: u64,
    pub title: String,
    /// "opened" | "merged" | "closed" | "locked"
    pub state: String,
    pub author: String,
    pub source_branch: String,
    pub target_branch: String,
    /// unix seconds
    pub created_at: i64,
    pub updated_at: i64,
    pub merged_at: Option<i64>,
    pub web_url: String,
    pub description: String,
    /// SHA HEAD de la rama origen.
    pub sha: Option<String>,
    pub merge_commit_sha: Option<String>,
    pub squash_commit_sha: Option<String>,
    pub draft: bool,
    /// Estado del último pipeline si viene en la respuesta ("success", "failed"...).
    pub pipeline_status: Option<String>,
    pub labels: Vec<String>,
    pub user_notes_count: u64,
}

#[derive(Debug, Clone, Default)]
pub struct MrFileDiff {
    pub old_path: String,
    pub new_path: String,
    pub new_file: bool,
    pub deleted_file: bool,
    pub renamed_file: bool,
    /// Diff unificado (solo hunks, como lo da GitLab).
    pub diff: String,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Default)]
pub struct MrCommit {
    pub id: String,
    pub short: String,
    pub title: String,
    pub author: String,
    pub time: i64,
}

/// Candidato a proyecto para el selector inicial.
#[derive(Debug, Clone, Default)]
pub struct Candidate {
    pub path: PathBuf,
    /// Nombre relativo a la carpeta de partida (p. ej. "dirtecnica/intranet-dirtecnica-backend").
    pub name: String,
    pub score: f64,
    /// true si hay señales de que un agente/IA ha trabajado ahí recientemente.
    pub active: bool,
    /// Motivos legibles: "agente claude en w3:p1", "12 ficheros modificados"...
    pub reasons: Vec<String>,
    /// unix seconds de la última actividad detectada.
    pub last_activity: Option<i64>,
    pub branch: Option<String>,
    pub dirty: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AiKind {
    /// Commit completo: qué cambia y por qué.
    Commit,
    /// Un fichero dentro de un commit o MR: qué ha cambiado en él.
    File,
    /// Merge Request completo: propósito + resumen por fichero.
    MergeRequest,
}

#[derive(Debug, Clone)]
pub struct AiRequest {
    /// Clave estable para caché (p. ej. "commit:<sha>", "mr:<host/proyecto>:<iid>:<sha>", "file:<sha>:<path>").
    pub cache_key: String,
    pub kind: AiKind,
    /// Título corto (subject del commit, título del MR, ruta del fichero).
    pub title: String,
    /// Metadatos en texto libre (autor, ramas, descripción del MR, lista de ficheros...).
    pub context: String,
    /// Diff unificado (se trunca según config).
    pub diff: String,
}
