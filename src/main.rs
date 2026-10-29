use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    str::FromStr,
};

use clap::{error::ErrorKind, Parser};
use color_eyre::eyre::{eyre, Context};
use dialoguer::{theme::ColorfulTheme, FuzzySelect, MultiSelect};
use entry::Entry;

use crate::entry::generate_expanded_entries;

mod entry;

const DATA_FILENAME: &str = "projects.json";
const MAX_SESSION_NAME_LEN: usize = 24;

/// Derives a tmux/zellij session name from a path's file name, trimmed to
/// `MAX_SESSION_NAME_LEN` characters.
fn session_name_for_path(path: &Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().chars().take(MAX_SESSION_NAME_LEN).collect())
}

#[derive(Debug, Clone, Copy, Default)]
enum Multiplexer {
    Tmux,
    #[default]
    Zellij,
}

impl FromStr for Multiplexer {
    type Err = color_eyre::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "tmux" => Ok(Multiplexer::Tmux),
            "zellij" => Ok(Multiplexer::Zellij),
            other => Err(eyre!(
                "unknown multiplexer `{other}`; expected `tmux` or `zellij`"
            )),
        }
    }
}

fn multiplexer() -> color_eyre::Result<Multiplexer> {
    match std::env::var("OPEN_PROJECT_MULTIPLEXER") {
        Ok(val) => val.parse(),
        Err(_) => Ok(Multiplexer::default()),
    }
}

/// Cli to open projects easily easily without needing to care for the working directory
#[derive(Parser, Debug)]
#[clap(version)]
enum Cli {
    Open,
    OpenTerm {
        #[clap(short, long)]
        new_window: bool,
    },
    OpenGui {
        #[clap(short, long)]
        new_window: bool,
    },
    List,
    Add {
        path: PathBuf,

        /// zellij layout to use when opening this entry (default: "project")
        #[clap(short, long)]
        zellij_layout: Option<String>,

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

            if let Some(name) = selected_entry.path.file_name() {
                set_terminal_title(&name.to_string_lossy());
                std::io::stderr().flush()?;
            }

            match multiplexer()? {
                Multiplexer::Tmux => open_tmux_session(&selected_entry.path)?,
                Multiplexer::Zellij => open_zellij_session(&selected_entry.path, selected_entry.zellij_layout())?,
            }

            Ok(())
        }
        Cli::OpenTerm { new_window } => {
            let entries = generate_expanded_entries(entries)?;

            let selection = FuzzySelect::with_theme(&ColorfulTheme::default())
                .items(&entries)
                .interact_opt()?
                .unwrap_or_else(|| std::process::exit(1));

            let selected_entry = &entries[selection];

            wezterm_open_path_in_tab(selected_entry, new_window)?;

            Ok(())
        }
        Cli::OpenGui { new_window } => {
            let entries: HashMap<String, Entry> = generate_expanded_entries(entries)?
                .into_iter()
                .map(|entry| (entry.to_string(), entry))
                .collect();

            let mut chooser = if cfg!(target_os = "linux") {
                let mut anyrun = Command::new("anyrun");
                anyrun.args([
                    "--plugins",
                    "libstdin.so",
                    "--show-results-immediately",
                    "true",
                ]);
                anyrun
            } else if cfg!(target_os = "macos") {
                Command::new("choose")
            } else {
                panic!("unsupported os");
            };

            let mut chooser = chooser
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()?;

            let mut chooser_stdin = chooser
                .stdin
                .take()
                .expect("should be able to take stdin of rofi");

            for entry in &entries {
                writeln!(chooser_stdin, "{}", entry.0)?;
            }
            drop(chooser_stdin);

            let selected_str = String::from_utf8(chooser.wait_with_output()?.stdout)?;
            let selected_str = selected_str.trim();

            if selected_str.is_empty() {
                std::process::exit(1);
            }

            let selected_entry = entries
                .get(selected_str)
                .ok_or(eyre!("unknown entry (`{selected_str}`) got selected"))?;

            wezterm_open_path_in_tab(selected_entry, new_window)?;

            Ok(())
        }
        Cli::List => {
            println!("{}", serde_json::to_string_pretty(&entries)?);
            Ok(())
        }
        Cli::Add { path, zellij_layout, prepend } => {
            let path = PathBuf::from_str(&shellexpand::tilde(
                path.to_str().ok_or(eyre!("expected valid utf-8 path"))?,
            ))?;

            let entry = Entry { path, zellij_layout };
            if prepend {
                entries.push_front(entry);
            } else {
                entries.push_back(entry);
            }

            serde_json::to_writer_pretty(File::create(&entries_filepath)?, &entries)?;

            Ok(())
        }
        Cli::Remove { path } => {
            if let Some(path) = path {
                entries.retain(|entry| entry.path != path);
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

fn wezterm_open_path_in_tab(entry: &Entry, new_window: bool) -> color_eyre::Result<()> {
    let path = &entry.path;
    let mut command = Command::new("wezterm");
    command
        .current_dir(path)
        .args(["cli", "spawn", "--cwd"])
        .arg(path);

    if new_window {
        command.arg("--new-window");
    }

    match multiplexer()? {
        Multiplexer::Tmux => {
            command.arg("tmux");
            if let Some(name) = session_name_for_path(path) {
                if tmux_session_exists(&name)? {
                    command.args(["a", "-t"]);
                    command.arg(&name);
                } else {
                    command.args(["new", "-s"]);
                    command.arg(&name);
                }
            }
        }
        Multiplexer::Zellij => {
            command.arg("zellij");
            if let Some(name) = session_name_for_path(path) {
                if zellij_session_exists(&name)? {
                    command.args(["attach", &name]);
                } else {
                    command.args(["-s", &name]);
                    command.args(["-n", entry.zellij_layout()]);
                }
            }
        }
    }

    let output = command.output()?;
    if !output.status.success() {
        eprintln!("failed to spawn tab: {}", output.status);
        return Ok(());
    }

    if let Some(name) = path.file_name() {
        let pane_id = String::from_utf8_lossy(&output.stdout);
        let pane_id = pane_id.trim();
        if !pane_id.is_empty() {
            let status = Command::new("wezterm")
                .args(["cli", "set-tab-title", "--pane-id", pane_id])
                .arg(name)
                .status()?;
            if !status.success() {
                eprintln!("failed to set tab title: {status}");
            }
        }
    }

    Ok(())
}

fn open_tmux_session(path: &Path) -> color_eyre::Result<()> {
    let mut command = Command::new("tmux");

    if let Some(name) = session_name_for_path(path) {
        if tmux_session_exists(&name)? {
            command.args(["a", "-t"]);
            command.arg(&name);
        } else {
            command.args(["new", "-s"]);
            command.arg(&name);
            command.arg("-c");
            command.arg(path);
        }
    }

    let status = command.spawn()?.wait()?;
    if !status.success() {
        eprintln!("failed to open tmux session: {status}");
    };

    Ok(())
}

fn tmux_session_exists(session_name: &str) -> color_eyre::Result<bool> {
    Ok(String::from_utf8(
        Command::new("tmux")
            .args(["ls", "-F", "#{session_name}"])
            .output()?
            .stdout,
    )
    .wrap_err("expected tmux ls to output valid utf-8")?
    .lines()
    .any(|existing| session_name == existing.trim()))
}

fn open_zellij_session(path: &Path, layout: &str) -> color_eyre::Result<()> {
    let mut command = Command::new("zellij");
    command.current_dir(path);

    if let Some(name) = session_name_for_path(path) {
        if zellij_session_exists(&name)? {
            command.arg("attach");
            command.arg(&name);
        } else {
            command.args(["-s", &name]);
            command.args(["-n", layout]);
        }
    }

    let status = command.spawn()?.wait()?;
    if !status.success() {
        eprintln!("failed to open zellij session: {status}");
    }

    Ok(())
}

fn zellij_session_exists(session_name: &str) -> color_eyre::Result<bool> {
    let current = std::env::var("ZELLIJ_SESSION_NAME").unwrap_or_default();
    Ok(String::from_utf8(
        Command::new("zellij")
            .args(["ls", "-s"])
            .output()?
            .stdout,
    )
    .wrap_err("expected zellij ls to output valid utf-8")?
    .lines()
    .any(|line| line.trim() == session_name && line.trim() != current))
}

fn set_terminal_title(title: &str) {
    eprint!("\x1B]0;{}\x07", title);
}
