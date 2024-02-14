use std::{collections::VecDeque, fs::File, path::PathBuf, process::Command, str::FromStr};

use clap::{error::ErrorKind, Parser};
use color_eyre::eyre::eyre;
use dialoguer::{theme::ColorfulTheme, FuzzySelect, MultiSelect};
use entry::Entry;
use winnow::Parser as _;

use crate::{entry::generate_expanded_entries, session::parse_zellij_ls};

mod entry;
mod session;

const DATA_FILENAME: &str = "projects.json";
const DEFAULT_LAYOUT: &str = "dev-default";

/// Cli to open projects easily easily without needing to care for the working directory
/// currently: open a new wezterm tab and open `zellij -l=<layout>` inside it
#[derive(Parser, Debug)]
enum Cli {
    Open,
    List,
    Add {
        path: PathBuf,

        /// layout to pass to zellij -l=<layout> when opening this path
        layout: Option<String>,

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

            let (path, layout) = match selected_entry {
                Entry::JustPath(path) => (path, DEFAULT_LAYOUT),
                Entry::PathWithlayout { path, layout } => (path, layout.as_str()),
            };

            let mut command = Command::new("wezterm");
            command
                .current_dir(path)
                .args(["cli", "spawn", "--cwd"])
                .arg(path)
                .args(["zellij", "-l"])
                .arg(layout);

            if let Some(name) = path.file_name() {
                let sessions = Command::new("zellij").args(["ls", "-n"]).output()?;
                let sessions = String::from_utf8(sessions.stdout)?;

                let sessions = parse_zellij_ls.parse(&sessions).map_err(|err| {
                    eyre!(
                        "offset: {}\ncontext: {:?}\n{err}",
                        err.offset(),
                        err.inner().context().collect::<Vec<_>>()
                    )
                })?;
                if let Some(existing_sesion) = sessions
                    .into_iter()
                    .find(|session| session.name == name.to_string_lossy())
                {
                    if existing_sesion.exited {
                        Command::new("zellij")
                            .arg("delete-session")
                            .arg(name)
                            .status()?;
                        command.arg("-s").arg(name);
                    }
                } else {
                    command.arg("-s").arg(name);
                }
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
        Cli::Add {
            path,
            layout,
            prepend,
        } => {
            let path = PathBuf::from_str(&shellexpand::tilde(
                path.to_str().ok_or(eyre!("expected valid utf-8 path"))?,
            ))?;
            let new_entry = if let Some(layout) = layout {
                Entry::PathWithlayout { path, layout }
            } else {
                Entry::JustPath(path)
            };

            if prepend {
                entries.push_front(new_entry);
            } else {
                entries.push_back(new_entry);
            }

            serde_json::to_writer_pretty(File::create(&entries_filepath)?, &entries)?;

            Ok(())
        }
        Cli::Remove { path } => {
            if let Some(path) = path {
                entries.retain(|entry| *entry.get_path() != path);
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
