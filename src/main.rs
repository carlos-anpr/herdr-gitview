mod ai;
mod app;
mod config;
mod diff;
mod discover;
#[allow(dead_code)] // API completa del contrato; no todo lo usa la UI
mod git;
#[allow(dead_code)] // API completa del contrato; no todo lo usa la UI
mod gitlab;
mod graph;
#[allow(dead_code)] // API completa del contrato; no todo lo usa la UI
mod model;
mod text;
mod ui;

use app::App;
use discover::{Discovery, HerdrCtx};
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;
use std::path::PathBuf;
use std::time::Duration;

const USAGE: &str = "herdr-gitview [RUTA] [--pick] [--discover]

Visor de commits, ramas y Merge Requests de GitLab con explicaciones de IA.
  RUTA         carpeta de partida (por defecto: cwd del panel activo de herdr o el cwd)
  --pick       mostrar siempre el selector de proyectos
  --discover   imprimir los candidatos detectados y salir (diagnóstico)";

fn main() -> anyhow::Result<()> {
    let mut path: Option<PathBuf> = None;
    let (mut pick, mut print) = (false, false);
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--pick" => pick = true,
            "--discover" => print = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("herdr-gitview {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => path = Some(PathBuf::from(a)),
        }
    }
    let cfg = config::load();
    let ctx = HerdrCtx::from_env();
    let start = path
        .or_else(|| ctx.pane_cwd.clone())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let start = start.canonicalize().unwrap_or(start);

    let found = if pick {
        let base = git::repo_root(&start).and_then(|r| r.parent().map(PathBuf::from)).unwrap_or(start.clone());
        Discovery::Pick(discover::rank(&base, &discover::find_repos(&base, cfg.scan_depth), &cfg, &ctx))
    } else {
        discover::discover(&start, &cfg, &ctx)
    };

    if print {
        match &found {
            Discovery::Direct(p) => println!("directo: {}", p.display()),
            Discovery::Pick(c) => {
                for c in c {
                    println!("{} {:6.1} {:40} {}", if c.active { "●" } else { "○" }, c.score, c.name, c.reasons.join(" · "));
                }
            }
        }
        return Ok(());
    }

    let mut app = App::new(cfg);
    match found {
        Discovery::Direct(p) => app.open_repo_at(p, &start),
        Discovery::Pick(c) if c.is_empty() => {
            eprintln!("No hay repositorios git en {}", start.display());
            return Ok(());
        }
        Discovery::Pick(c) => app.open_picker(start, c),
    }

    let mut term = ratatui::init();
    execute!(std::io::stdout(), EnableMouseCapture)?;
    let res = run(&mut term, &mut app);
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    res
}

fn run(term: &mut ratatui::DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    while !app.quit {
        term.draw(|f| ui::draw(f, app))?;
        if event::poll(Duration::from_millis(60))? {
            app.on_event(event::read()?);
            // Vaciar ráfagas (rueda del ratón, tecla mantenida) antes de repintar.
            while event::poll(Duration::ZERO)? {
                app.on_event(event::read()?);
                if app.quit {
                    break;
                }
            }
        }
        while let Ok(m) = app.rx.try_recv() {
            app.on_msg(m);
        }
        app.tick();
    }
    Ok(())
}
