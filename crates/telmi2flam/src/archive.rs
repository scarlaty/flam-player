//! Archive Telmi : ouverture, index des noms (`_fold`), racine de l'histoire (`find_root`) et
//! garde des noms d'assets (`check_asset_name`), comme dans `telmi2flam.py`.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::Path;

use unicode_normalization::UnicodeNormalization;

use crate::error::ConvError;
use crate::zipread::{ZipError, ZipReader};

/// `_fold` : clé de recherche d'asset, NFC puis minuscules (`str.lower()`, sigma final et
/// U+0130 compris).
pub fn fold(name: &str) -> String {
    name.nfc().collect::<String>().to_lowercase()
}

/// Racine de l'histoire (`find_root`) : dossier qui contient `metadata.json` ET `nodes.json`
/// (noms exacts), hors `__MACOSX/` et fichiers `._*`. Le moins profond gagne ; plusieurs au
/// même niveau : erreur `invalid_content`.
pub fn find_root(names: &[String]) -> Result<String, ConvError> {
    let nameset: HashSet<&str> = names.iter().map(String::as_str).collect();
    let mut cands: Vec<&str> = Vec::new();
    for n in names {
        let parts: Vec<&str> = n.split('/').collect();
        if parts.last() != Some(&"metadata.json") || parts.contains(&"__MACOSX") {
            continue;
        }
        if parts.iter().any(|p| p.starts_with("._")) {
            continue;
        }
        let root = &n[..n.len() - "metadata.json".len()];
        if nameset.contains(format!("{root}nodes.json").as_str()) {
            cands.push(root);
        }
    }
    let Some(depth) = cands.iter().map(|r| r.matches('/').count()).min() else {
        return Err(ConvError::not_telmi(
            "metadata.json + nodes.json introuvables dans l'archive \
             (ce n'est pas une histoire TELMI ?)",
        ));
    };
    let mut best: Vec<&str> = cands
        .into_iter()
        .filter(|r| r.matches('/').count() == depth)
        .collect();
    best.sort_unstable();
    if best.len() > 1 {
        let list: Vec<&str> = best
            .iter()
            .map(|r| if r.is_empty() { "/" } else { r })
            .collect();
        return Err(ConvError::invalid_content(format!(
            "plusieurs histoires dans l'archive : {}",
            list.join(", ")
        )));
    }
    Ok(best[0].to_string())
}

/// `check_asset_name` : refuse les noms d'assets qui pourraient sortir du dossier (zip-slip).
pub fn check_asset_name<'a>(name: &'a str, what: &str) -> Result<&'a str, ConvError> {
    let parts: Vec<&str> = name.split('/').collect();
    if name.is_empty()
        || name.starts_with('/')
        || name.contains('\\')
        || name.contains(':')
        || name.contains('\0')
        || parts.contains(&"..")
        || parts.contains(&"")
    {
        return Err(ConvError::invalid_content(format!(
            "{what} : nom d'asset refuse (chemin dangereux) : {name:?}"
        )));
    }
    Ok(name)
}

/// Convertit un échec de lecture en erreur de conversion (code 3, ou 7 pour bzip2/LZMA).
fn conv(source: &str, e: ZipError) -> ConvError {
    match e {
        ZipError::Unsupported(m) => ConvError::unsupported_format(format!("{source} : {m}")),
        ZipError::Corrupt(m) | ZipError::Encrypted(m) | ZipError::PythonFailsToo(m) => {
            ConvError::archive_unreadable(format!("{source} : {m}"))
        }
    }
}

/// Archive Telmi ouverte : noms (`namelist()`), index replié et lecture des entrées.
pub struct Archive<R> {
    zip: ZipReader<R>,
    names: Vec<String>,
    /// `lower_index` : `setdefault(_fold(n), n)`, le premier nom rencontré gagne.
    folded: HashMap<String, String>,
    /// Nom de l'archive pour les messages d'erreur.
    source: String,
}

impl Archive<BufReader<File>> {
    /// `zipfile.ZipFile(path, "r")` : archive absente, illisible ou corrompue →
    /// `archive_unreadable` (code 3).
    pub fn open(path: &Path) -> Result<Self, ConvError> {
        let source = path.display().to_string();
        let file = File::open(path)
            .map_err(|e| ConvError::archive_unreadable(format!("{source} : {e}")))?;
        Archive::new(BufReader::new(file), source)
    }
}

impl<R: Read + Seek> Archive<R> {
    /// Ouvre une archive déjà en mémoire ou sur un autre support.
    pub fn new(reader: R, source: String) -> Result<Self, ConvError> {
        let zip = ZipReader::new(reader).map_err(|e| conv(&source, e))?;
        let names = zip.namelist();
        let mut folded = HashMap::new();
        for n in &names {
            folded.entry(fold(n)).or_insert_with(|| n.clone());
        }
        Ok(Archive {
            zip,
            names,
            folded,
            source,
        })
    }

    /// `namelist()` : doublons compris, dans l'ordre du répertoire central.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Un nom replié (`_fold`) est-il présent ?
    pub fn contains_folded(&self, folded_name: &str) -> bool {
        self.folded.contains_key(folded_name)
    }

    /// `lookup` de `_convert` : nom exact, sinon premier nom de même `_fold`.
    pub fn lookup(&self, full: &str) -> Option<String> {
        if self.zip.contains(full) {
            return Some(full.to_string());
        }
        self.folded.get(&fold(full)).cloned()
    }

    /// `zf.read(name)` : la dernière entrée de ce nom (`getinfo`), lue jusqu'au bout et CRC
    /// vérifié. `Ok(None)` si le nom est absent.
    pub fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>, ConvError> {
        let source = self.source.clone();
        self.zip.read(name).map_err(|e| conv(&source, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn fold_nfc_puis_minuscules() {
        assert_eq!(fold("E\u{301}TÉ.PNG"), "été.png");
        assert_eq!(fold("\u{130}"), "i\u{307}");
        // Sigma final : « ΟΔΟΣ » → « οδος » (ς final, comme Python).
        assert_eq!(
            fold("\u{39f}\u{394}\u{39f}\u{3a3}"),
            "\u{3bf}\u{3b4}\u{3bf}\u{3c2}"
        );
    }

    #[test]
    fn find_root_choisit_le_moins_profond() {
        let n = names(&[
            "a/b/metadata.json",
            "a/b/nodes.json",
            "a/metadata.json",
            "a/nodes.json",
        ]);
        assert_eq!(find_root(&n).unwrap(), "a/");
    }

    #[test]
    fn find_root_ignore_macosx_et_points_soulignes() {
        let n = names(&[
            "__MACOSX/metadata.json",
            "__MACOSX/nodes.json",
            "x/._metadata.json",
            "x/._nodes.json",
            "metadata.json",
            "nodes.json",
        ]);
        assert_eq!(find_root(&n).unwrap(), "");
    }

    #[test]
    fn find_root_erreurs() {
        let e = find_root(&names(&["readme.txt", "metadata.json"])).unwrap_err();
        assert_eq!((e.code, e.exit_code), ("not_telmi", 3));
        let e = find_root(&names(&[
            "b/metadata.json",
            "b/nodes.json",
            "a/metadata.json",
            "a/nodes.json",
        ]))
        .unwrap_err();
        assert_eq!((e.code, e.exit_code), ("invalid_content", 4));
        assert!(e.message.ends_with("a/, b/"), "{}", e.message);
        // Nom exact : la casse compte pour la racine.
        let e = find_root(&names(&["Metadata.json", "nodes.json"])).unwrap_err();
        assert_eq!(e.code, "not_telmi");
    }

    #[test]
    fn check_asset_name_refuse_les_chemins_dangereux() {
        for bad in [
            "", "/a.png", "a\\b.png", "c:x.png", "a\0.png", "../a.png", "a/../b", "a//b", "a/",
        ] {
            let e = check_asset_name(bad, "test").unwrap_err();
            assert_eq!((e.code, e.exit_code), ("invalid_content", 4), "{bad:?}");
        }
        for ok in ["a.png", "img/a.png", "été/b.mp3", "..a/b", "a..png"] {
            assert_eq!(check_asset_name(ok, "test").unwrap(), ok);
        }
    }
}
