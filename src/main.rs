use std::{collections::VecDeque, fs::File, path::PathBuf, process::Command, str::FromStr};

use clap::{error::ErrorKind, Parser};
use color_eyre::eyre::eyre;
use dialoguer::{theme::ColorfulTheme, FuzzySelect, MultiSelect};
use entry::Entry;

use crate::entry::generate_expanded_entries;

mod entry;

const DATA_FILENAME: &str = "projects.json";

/// Cli to open projects easily easily without needing to care for the working directory
/// currently: open a new wezterm tab and open `zellij -l=<layout>` inside it
#[derive(Parser, Debug)]
enum Cli {
    Open,
    List,
    Add {
        path: PathBuf,

        /// add it to the start of the list, giving it a higher priority
        #[clap(short, long)]
        prepend: bool,
    },
    Remove {
        path: Option<PathBuf>,
    },
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => match err.kind() {
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => Cli::Open,
            _ => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        },
    };

    let project_dirs = directories::ProjectDirs::from("", "", "open-project-cli")
        .ok_or(eyre!("unable to valid home directory path"))?;
    let entries_filepath = project_dirs.data_dir().join(DATA_FILENAME);

    if !entries_filepath.try_exists()? {
        std::fs::create_dir_all(
            entries_filepath
                .parent()
                .expect("should have a valid data directory"),
        )?;
        std::fs::write(&entries_filepath, "[]")?;
    }

    let mut entries: VecDeque<Entry> = serde_json::from_reader(File::open(&entries_filepath)?)?;

    match cli {
        Cli::Open => {
            let entries = generate_expanded_entries(entries)?;

            let selection = FuzzySelect::with_theme(&ColorfulTheme::default())
                .items(&entries)
                .interact_opt()?
                .unwrap_or_else(|| std::process::exit(1));

            let selected_entry = &entries[selection];

            let path = &selected_entry.0;

            let mut command = Command::new("wezterm");
            // do session name based on last part of path
            command
                .current_dir(path)
                .args(["cli", "spawn", "--cwd"])
                .arg(path)
                .args(["tmux", "new"]);

            if let Some(name) = path.file_name() {
                command.arg("-s").arg(name);
            }

            let status = command.spawn()?.wait()?;

            if !status.success() {
                eprintln!("failed to spawn tab: {status}");
            }

            Ok(())
        }
        Cli::List => {
            println!("{}", serde_json::to_string_pretty(&entries)?);
            Ok(())
        }
        Cli::Add { path, prepend } => {
            let path = PathBuf::from_str(&shellexpand::tilde(
                path.to_str().ok_or(eyre!("expected valid utf-8 path"))?,
            ))?;

            if prepend {
                entries.push_front(Entry(path));
            } else {
                entries.push_back(Entry(path));
            }

            serde_json::to_writer_pretty(File::create(&entries_filepath)?, &entries)?;

            Ok(())
        }
        Cli::Remove { path } => {
            if let Some(path) = path {
                entries.retain(|entry| *entry.0 != path);
            } else {
                let mut selected_entries = MultiSelect::with_theme(&ColorfulTheme::default())
                    .items(entries.make_contiguous())
                    .interact_opt()?
                    .unwrap_or_else(|| std::process::exit(1));
                selected_entries.sort();

                selected_entries.iter().rev().for_each(|idx| {
                    entries.remove(*idx);
                });
            }

            serde_json::to_writer_pretty(File::create(&entries_filepath)?, &entries)?;

            Ok(())
        }
    }
}
