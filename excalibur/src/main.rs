use crate::app::App;
use crate::modules::ModuleId;
use clap::{Parser, Subcommand};
use std::fs::OpenOptions;

pub mod app;
pub mod event;
pub mod modules;
pub mod ui;
pub mod view;

#[derive(Parser)]
#[command(name = "excalibur")]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Browse and search shell command history
    #[command(visible_alias = "h")]
    History,

    /// Inspect running processes and their supervisors (Linux only)
    #[cfg(target_os = "linux")]
    #[command(visible_alias = "pt")]
    ProcessTracer,

    /// Switch Claude Code settings profiles
    #[command(visible_alias = "s")]
    Settings,

    /// Manage ssh config and port-forward tunnels
    #[command(visible_alias = "t")]
    Ssh,
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    // Parse CLI arguments
    let cli = Cli::parse();

    // Determine initial module (if any)
    let initial_module = match cli.command {
        Some(Commands::History) => Some(ModuleId::History),
        #[cfg(target_os = "linux")]
        Some(Commands::ProcessTracer) => Some(ModuleId::ProcessTracer),
        Some(Commands::Settings) => Some(ModuleId::Settings),
        Some(Commands::Ssh) => Some(ModuleId::Ssh),
        None => None,
    };

    // Open /dev/tty for terminal I/O instead of stdout
    // This allows stdout to be used for command output while TUI uses /dev/tty
    let tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;

    let backend = ratatui::backend::CrosstermBackend::new(tty);
    let mut terminal = ratatui::Terminal::new(backend)?;

    // Setup terminal
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;

    // Run app with or without initial module
    let result = match initial_module {
        Some(module_id) => App::new_with_module(module_id).run(&mut terminal),
        None => App::new().run(&mut terminal),
    };

    // Restore terminal
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;

    result
}
