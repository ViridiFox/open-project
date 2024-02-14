use std::{path::PathBuf, fmt::Display, collections::{VecDeque, HashSet}};

use color_eyre::eyre::eyre;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(untagged)]
pub enum Entry {
    JustPath(PathBuf),
    PathWithlayout { path: PathBuf, layout: String },
}

impl Entry {
    pub fn get_path(&self) -> &PathBuf {
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

pub fn generate_expanded_entries(entries: VecDeque<Entry>) -> color_eyre::Result<Vec<Entry>> {
    let mut res = Vec::with_capacity(entries.len());

    let mut seen_paths = HashSet::new();

    for entry in entries {
        let path = entry.get_path().to_str().ok_or(eyre!("path '{:?}' is not valid utf-8", entry.get_path()))?;
        let paths = glob::glob(path)?;

        for path in paths.filter_map(Result::ok) {
            if seen_paths.insert(path.clone()) {
                let entry = entry.clone().with_path(path);
                res.push(entry);
            }
        }
    }

    Ok(res)
}
