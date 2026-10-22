//! Tipi delle operazioni sui file, condivisi col frontend via IPC (camelCase, tag `"type"`).

use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use crate::vfs::loc::Loc;

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

/// Per Copy/Move `dest` è la cartella di destinazione, salvo con un solo file come sorgente e una
/// `dest` che non è una cartella esistente né termina con `\`: allora è il nome del file da creare
/// (`plan::file_target`, regola di TC). Per Rename il nuovo percorso completo; per Mkdir il
/// percorso da creare è `sources[0]`. Sorgenti e destinazione sono posizioni, locali o remote (stringhe nel JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSpec {
    pub kind: JobKind,
    pub sources: Vec<Loc>,
    pub dest: Option<Loc>,
    /// Maschera TC applicata ai soli file.
    pub filter: Option<String>,
    pub policy: ConflictPolicy,
}

/// Elemento elementare di un'operazione. `size` è la dimensione per i file, 0 per le cartelle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileItem {
    pub src: Loc,
    pub dst: Option<Loc>,
    pub is_dir: bool,
    pub size: u64,
    /// Cartella vera il cui contenuto non è nell'elenco: si elenca durante l'esecuzione, se serve
    /// (illeggibile in `expand`, oppure da spostare o cestinare intera). Mai per file e collegamenti.
    pub unlisted: bool,
}

/// Voce di una cartella: quanto serve per espandere un job (passa anche dalla pipe dell'helper).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListEntry {
    /// `OsString`: anche un nome non valido in UTF-16 (surrogato spaiato) passa intatto.
    pub name: OsString,
    /// Cartella vera, da attraversare; falso per giunzioni e symlink.
    pub dir: bool,
    /// Giunzione o symlink a una cartella: una foglia trattata come cartella.
    pub dir_link: bool,
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
    Rename(Loc),
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
            sources: vec![Loc::Local(r"C:\a".into())],
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

        let r = Decision::Rename(Loc::Local(r"C:\f (2).txt".into()));
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v, json!({"type": "rename", "path": r"C:\f (2).txt"}));
        assert_eq!(serde_json::from_value::<Decision>(v).unwrap(), r);
        assert_eq!(serde_json::to_value(Decision::Skip).unwrap(), json!({"type": "skip"}));

        let item = FileItem { src: Loc::Local("a".into()), dst: None, is_dir: true, size: 0, unlisted: false };
        assert_eq!(
            serde_json::to_value(&item).unwrap(),
            json!({"src": "a", "dst": null, "isDir": true, "size": 0, "unlisted": false})
        );
        let m = Meta { size: 1, modified_ms: 2 };
        assert_eq!(serde_json::to_value(m).unwrap(), json!({"size": 1, "modifiedMs": 2}));
    }
}
