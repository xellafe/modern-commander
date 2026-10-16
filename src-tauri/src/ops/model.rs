//! Tipi delle operazioni sui file, condivisi col frontend via IPC (camelCase, tag `"type"`).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum JobKind {
    Copy,
    Move,
    Delete { permanent: bool },
    Mkdir,
    Rename,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ConflictPolicy {
    Ask,
    Overwrite,
    Skip,
    OverwriteIfNewer,
    OverwriteIfLarger,
    RenameNew,
}

/// Per Copy/Move `dest` è la cartella di destinazione; per Rename il nuovo percorso completo;
/// per Mkdir il percorso da creare è `sources[0]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSpec {
    pub kind: JobKind,
    pub sources: Vec<PathBuf>,
    pub dest: Option<PathBuf>,
    /// Maschera TC applicata ai soli file.
    pub filter: Option<String>,
    pub policy: ConflictPolicy,
}

/// Elemento elementare di un'operazione. `size` è la dimensione per i file, 0 per le cartelle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileItem {
    pub src: PathBuf,
    pub dst: Option<PathBuf>,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub size: u64,
    pub modified_ms: i64,
}

/// `Rename` porta il nuovo percorso completo di destinazione.
/// Il contenuto va in `path`: un enum con solo `tag` non può serializzare una variante con stringa.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "path", rename_all = "camelCase")]
pub enum Decision {
    Overwrite,
    Skip,
    Rename(PathBuf),
    Cancel,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serde_shapes() {
        let spec = JobSpec {
            kind: JobKind::Delete { permanent: true },
            sources: vec![PathBuf::from(r"C:\a")],
            dest: None,
            filter: None,
            policy: ConflictPolicy::OverwriteIfNewer,
        };
        let v = serde_json::to_value(&spec).unwrap();
        assert_eq!(
            v,
            json!({"kind": {"type": "delete", "permanent": true}, "sources": [r"C:\a"], "dest": null,
                   "filter": null, "policy": {"type": "overwriteIfNewer"}})
        );
        assert_eq!(serde_json::from_value::<JobSpec>(v).unwrap(), spec);

        let r = Decision::Rename(PathBuf::from(r"C:\f (2).txt"));
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v, json!({"type": "rename", "path": r"C:\f (2).txt"}));
        assert_eq!(serde_json::from_value::<Decision>(v).unwrap(), r);
        assert_eq!(serde_json::to_value(Decision::Skip).unwrap(), json!({"type": "skip"}));

        let item = FileItem { src: PathBuf::from("a"), dst: None, is_dir: true, size: 0 };
        assert_eq!(serde_json::to_value(&item).unwrap(), json!({"src": "a", "dst": null, "isDir": true, "size": 0}));
        let m = Meta { size: 1, modified_ms: 2 };
        assert_eq!(serde_json::to_value(m).unwrap(), json!({"size": 1, "modifiedMs": 2}));
    }
}
