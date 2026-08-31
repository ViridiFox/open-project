use std::{
    collections::{HashSet, VecDeque},
    fmt::Display,
    path::PathBuf,
};

use color_eyre::eyre::eyre;
use serde::{Deserialize, Deserializer, Serialize};

const DEFAULT_ZELLIJ_LAYOUT: &str = "project";

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Entry {
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zellij_layout: Option<String>,
}

impl Entry {
    pub fn zellij_layout(&self) -> &str {
        self.zellij_layout
            .as_deref()
            .unwrap_or(DEFAULT_ZELLIJ_LAYOUT)
    }

    fn with_path(mut self, path: PathBuf) -> Entry {
        self.path = path;
        self
    }
}

impl<'de> Deserialize<'de> for Entry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            JustPath(PathBuf),
            Full {
                path: PathBuf,
                zellij_layout: Option<String>,
            },
        }

        match Raw::deserialize(deserializer)? {
            Raw::JustPath(path) => Ok(Entry {
                path,
                zellij_layout: None,
            }),
            Raw::Full { path, zellij_layout } => Ok(Entry { path, zellij_layout }),
        }
    }
}

impl Display for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.path)
    }
}

pub fn generate_expanded_entries(entries: VecDeque<Entry>) -> color_eyre::Result<Vec<Entry>> {
    let mut res = Vec::with_capacity(entries.len());

    let mut seen_paths = HashSet::new();

    for entry in entries {
        let path = entry
            .path
            .to_str()
            .ok_or(eyre!("path '{:?}' is not valid utf-8", entry.path))?;
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
