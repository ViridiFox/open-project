use std::{
    collections::{HashSet, VecDeque},
    fmt::Display,
    path::PathBuf,
};

use color_eyre::eyre::eyre;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Entry(pub PathBuf);

impl Entry {
    fn with_path(mut self, path: PathBuf) -> Entry {
        self.0 = path;
        self
    }
}

impl Display for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

pub fn generate_expanded_entries(entries: VecDeque<Entry>) -> color_eyre::Result<Vec<Entry>> {
    let mut res = Vec::with_capacity(entries.len());

    let mut seen_paths = HashSet::new();

    for entry in entries {
        let path = entry
            .0
            .to_str()
            .ok_or(eyre!("path '{:?}' is not valid utf-8", entry.0))?;
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
