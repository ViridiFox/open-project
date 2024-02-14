use std::{
    collections::{HashSet, VecDeque},
    fmt::Display,
    fs::File,
    path::PathBuf,
    process::Command,
    str::FromStr,
    thread,
};

use clap::{error::ErrorKind, Parser};
use color_eyre::eyre::eyre;
use serde::{Deserialize, Serialize};
use skim::prelude::*;
use winnow::{
    ascii::{alphanumeric1, line_ending, multispace1, space0},
    combinator::{alt, delimited, empty, opt, repeat, separated},
    error::StrContext,
    prelude::*,
    token::literal,
    Parser as _,
};

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
            let skim_options = SkimOptionsBuilder::default()
                .build()
                .expect("expected valid skim options");

            let entry_receiver = send_expanded_entries(entries.clone());
            let output = Skim::run_with(&skim_options, Some(entry_receiver))
                .ok_or(eyre!("skim produced an error"))?;

            if output.is_abort {
                std::process::exit(1);
            }

            let selected = output
                .selected_items
                .iter()
                .map(|item| {
                    (**item)
                        .as_any()
                        .downcast_ref::<Entry>()
                        .expect("expected downcasting to work")
                })
                .collect::<Vec<&Entry>>();
            debug_assert_eq!(
                selected.len(),
                1,
                "skim_options.multi should be false and prevent multiple selected items"
            );

            let entry = selected[0];

            let (path, layout) = match entry {
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
                let mut sessions = String::from_utf8(sessions.stdout)?;

                let sessions = parse_zellij_ls.parse(&mut sessions).map_err(|err| {
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
                let skim_options = SkimOptionsBuilder::default()
                    .multi(true)
                    .bind(vec!["enter:abort"])
                    .build()
                    .expect("expected valid skim options");

                let entry_receiver = send_entries(entries.clone());

                let output = Skim::run_with(&skim_options, Some(entry_receiver))
                    .ok_or(eyre!("skim produced an error"))?;

                if output.is_abort {
                    std::process::exit(1);
                }

                let selected = output.selected_items.iter().map(|item| {
                    (**item)
                        .as_any()
                        .downcast_ref::<Entry>()
                        .expect("expected downcasting to work")
                });

                entries.retain(|entry| {
                    for item in selected.clone() {
                        if item == entry {
                            return false;
                        }
                    }
                    true
                });
            }

            serde_json::to_writer_pretty(File::create(&entries_filepath)?, &entries)?;

            Ok(())
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(untagged)]
enum Entry {
    JustPath(PathBuf),
    PathWithlayout { path: PathBuf, layout: String },
}

impl Entry {
    fn get_path(&self) -> &PathBuf {
        match self {
            Self::JustPath(path) => path,
            Self::PathWithlayout { path, .. } => path,
        }
    }

    fn with_path(self, path: PathBuf) -> Entry {
        match self {
            Self::JustPath(_) => Self::JustPath(path),
            Self::PathWithlayout { layout, .. } => Self::PathWithlayout { path, layout },
        }
    }
}

impl Display for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Entry::JustPath(path) => {
                write!(f, "'{path:?}'")
            }
            Entry::PathWithlayout { path, layout } => {
                write!(f, "'{path:?}' with layout '{layout}'")
            }
        }
    }
}

impl SkimItem for Entry {
    fn text(&self) -> Cow<str> {
        Cow::from(format!("{self}"))
    }
}

fn send_entries(entries: VecDeque<Entry>) -> SkimItemReceiver {
    let (entry_sender, entry_receiver) = unbounded();
    thread::spawn(move || {
        for entry in entries {
            let _ = entry_sender.send(Arc::new(entry) as Arc<dyn SkimItem>);
        }
    });

    entry_receiver
}

fn send_expanded_entries(entries: VecDeque<Entry>) -> SkimItemReceiver {
    let (entry_sender, entry_receiver) = unbounded();
    thread::spawn(move || {
        let mut seen_paths = HashSet::new();

        for entry in entries {
            let path = entry.get_path().to_str().unwrap_or_else(|| {
                eprintln!("path '{:?}' contains invalid utf-8", entry.get_path());
                std::process::exit(1);
            });
            let paths = glob::glob(path).unwrap_or_else(|err| {
                eprintln!("{}", color_eyre::Report::new(err));
                std::process::exit(1);
            });

            for path in paths.filter_map(Result::ok) {
                if seen_paths.insert(path.clone()) {
                    let entry = entry.clone().with_path(path);
                    let _ = entry_sender.send(Arc::new(entry) as Arc<dyn SkimItem>);
                }
            }
        }
    });

    entry_receiver
}

#[derive(Debug)]
struct ZellijSession {
    name: String,
    exited: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Status {
    Exited,
    Else,
}

fn parse_zellij_ls(input: &mut &str) -> PResult<Vec<ZellijSession>> {
    fn session_name(input: &mut &str) -> PResult<String> {
        repeat(1.., alt((alphanumeric1, literal("-"), literal("_"))))
            .map(|text: Vec<&str>| text.join(""))
            .context(StrContext::Label("session name"))
            .parse_next(input)
    }

    fn status(input: &mut &str) -> PResult<Status> {
        alt((
            "EXITED - attach to resurrect".map(|_| Status::Exited),
            "current".map(|_| Status::Else),
            empty.value(Status::Else),
        ))
        .context(StrContext::Label("session status"))
        .parse_next(input)
    }

    (
        separated(
            ..,
            (
                session_name,
                ' ',
                delimited::<_, _, String, _, _, _, _, _>(
                    '[',
                    repeat(.., alt((alphanumeric1, multispace1))),
                    ']',
                ),
                space0,
                opt(delimited('(', status, ')')),
            )
                .map(|(name, _, _, _, opt_status)| ZellijSession {
                    name,
                    exited: opt_status
                        .map(|status| status == Status::Exited)
                        .unwrap_or(false),
                })
                .context(StrContext::Label("session entry")),
            line_ending,
        ),
        opt(line_ending),
    )
        .map(|res| res.0)
        .context(StrContext::Label("session list"))
        .parse_next(input)
}
