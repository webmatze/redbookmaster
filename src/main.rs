use clap::{Parser, Subcommand};
use colored::Colorize;
use std::path::PathBuf;

mod cli;
mod core;
mod audio;
mod export;
mod burn;

use cli::wizard;
use core::project::Project;

#[derive(Parser)]
#[command(name = "redbookmaster")]
#[command(author, version, about = "A CLI tool for creating Red Book compatible CD masters")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start a new project wizard
    New,
    /// Open an existing project
    Open {
        /// Path to the project file (.rbm)
        path: PathBuf,
    },
    /// Add WAV files to the current project
    Add {
        /// WAV files to add
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// List tracks in the current project
    List,
    /// Edit track or album metadata
    Edit {
        /// Track number to edit (omit for album metadata)
        track: Option<u8>,
    },
    /// Reorder tracks interactively
    Reorder,
    /// Configure track gaps
    Gaps,
    /// Play album or specific track
    Play {
        /// Track number to play (omit for full album)
        track: Option<u8>,
    },
    /// Play transition into a track
    Transition {
        /// Track number to transition into
        track: u8,
    },
    /// Export CUE/WAV master
    Export {
        /// Output directory
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Burn to CD
    Burn,
    /// Validate Red Book compliance
    Validate,
    /// Show project summary
    Info,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::New) => {
            wizard::new_project();
        }
        Some(Commands::Open { path }) => {
            match Project::load(&path) {
                Ok(project) => {
                    println!("{} Loaded project: {}", "✓".green(), project.album.title);
                    wizard::project_menu(project);
                }
                Err(e) => {
                    eprintln!("{} Failed to load project: {}", "✗".red(), e);
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::Add { files }) => {
            println!("Adding {} files...", files.len());
            // TODO: Implement add command
        }
        Some(Commands::List) => {
            // TODO: Implement list command
            println!("No project loaded. Use 'redbookmaster open <project>' first.");
        }
        Some(Commands::Edit { track }) => {
            match track {
                Some(n) => println!("Editing track {}...", n),
                None => println!("Editing album metadata..."),
            }
            // TODO: Implement edit command
        }
        Some(Commands::Reorder) => {
            // TODO: Implement reorder command
            println!("Reorder command not yet implemented");
        }
        Some(Commands::Gaps) => {
            // TODO: Implement gaps command
            println!("Gaps command not yet implemented");
        }
        Some(Commands::Play { track }) => {
            match track {
                Some(n) => println!("Playing track {}...", n),
                None => println!("Playing full album..."),
            }
            // TODO: Implement play command
        }
        Some(Commands::Transition { track }) => {
            println!("Playing transition into track {}...", track);
            // TODO: Implement transition command
        }
        Some(Commands::Export { output }) => {
            let out_dir = output.unwrap_or_else(|| PathBuf::from("."));
            println!("Exporting to {:?}...", out_dir);
            // TODO: Implement export command
        }
        Some(Commands::Burn) => {
            // TODO: Implement burn command
            println!("Burn command not yet implemented");
        }
        Some(Commands::Validate) => {
            // TODO: Implement validate command
            println!("Validate command not yet implemented");
        }
        Some(Commands::Info) => {
            // TODO: Implement info command
            println!("No project loaded. Use 'redbookmaster open <project>' first.");
        }
        None => {
            // No subcommand - launch interactive wizard
            wizard::main_menu();
        }
    }
}
