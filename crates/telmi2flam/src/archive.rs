//! Archive Telmi : ouverture, index des noms (`_fold`), racine de l'histoire (`find_root`) et
//! garde des noms d'assets (`check_asset_name`), comme dans l'ancien convertisseur Python.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::Path;

use unicode_normalization::UnicodeNormalization;

use crate::error::ConvError;
use crate::text::py_repr;
use crate::zipread::{system_io, ChunkStop, ZipError, ZipReader};

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
            "{what} : nom d'asset refuse (chemin dangereux) : {}",
            py_repr(name)
        )));
    }
    Ok(name)
}

/// Convertit un échec de lecture en erreur de conversion (`archive_unreadable`, code 3).
fn conv(source: &str, e: ZipError) -> ConvError {
    match e {
        ZipError::Corrupt(m) | ZipError::Encrypted(m) | ZipError::PythonFailsToo(m) => {
            ConvError::archive_unreadable(format!("{source} : {m}"))
        }
        ZipError::Io(m) => ConvError::archive_io(format!("{source} : {m}")),
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
        // Un dossier n'est pas une archive (Windows refuse son ouverture, Unix la lecture) :
        // `archive_unreadable`, pas une erreur du système.
        if path.is_dir() {
            return Err(ConvError::archive_unreadable(format!(
                "{source} : c'est un dossier"
            )));
        }
        let file = File::open(path).map_err(|e| {
            // Absente : `archive_unreadable` ; refusée par le système : `archive_io` (#580).
            if e.kind() != std::io::ErrorKind::NotFound && system_io(&e) {
                ConvError::archive_io(format!("{source} : {e}"))
            } else {
                ConvError::archive_unreadable(format!("{source} : {e}"))
            }
        })?;
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

    /// [`Archive::read`] avec annulation : `cancelled` est consulté pendant le décodage bzip2,
    /// LZMA ou Zstandard (voir `ZipReader::read_chunks`) ; s'il est levé,
    /// [`ConvError::cancelled`] est rendu. Même résultat que `read` sinon.
    pub fn read_cancellable(
        &mut self,
        name: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<u8>>, ConvError> {
        // `MAX_N` de `zipfile` : même découpage en tours que `read`.
        const MAX_N: usize = (1 << 31) - 1;
        let mut out = Vec::new();
        let found = self.read_chunks(
            name,
            MAX_N,
            cancelled,
            &ConvError::cancelled,
            &mut |c: &[u8]| {
                out.extend_from_slice(c);
                Ok::<(), ConvError>(())
            },
        )?;
        Ok(found.then_some(out))
    }

    /// [`Archive::read`] par tours d'au plus `chunk` octets passés à `sink` (CRC vérifié après
    /// le dernier). `Ok(false)` si le nom est absent ; une erreur de `sink` est rendue telle
    /// quelle, une erreur de lecture convertie comme pour `read`.
    /// `cancelled` est consulté pendant le décodage (voir `ZipReader::read_chunks`) ; s'il
    /// est levé, `stopped()` est rendu.
    pub fn read_chunks<E: From<ConvError>>(
        &mut self,
        name: &str,
        chunk: usize,
        cancelled: &dyn Fn() -> bool,
        stopped: &dyn Fn() -> E,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), E>,
    ) -> Result<bool, E> {
        self.zip
            .read_chunks(name, chunk, cancelled, sink)
            .map_err(|e| match e {
                ChunkStop::Zip(e) => E::from(conv(&self.source, e)),
                ChunkStop::Sink(e) => e,
                ChunkStop::Cancelled => stopped(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// Lecteur dont les lectures échouent, une fois `fail` levé, avec `error`.
    struct Failing {
        inner: std::io::Cursor<Vec<u8>>,
        fail: std::rc::Rc<std::cell::Cell<bool>>,
        error: fn() -> std::io::Error,
    }

    impl Read for Failing {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.fail.get() {
                return Err((self.error)());
            }
            self.inner.read(buf)
        }
    }

    impl Seek for Failing {
        fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(pos)
        }
    }

    /// #580 : une erreur du système (code de l'OS) à la lecture de l'archive rend `archive_io`,
    /// une archive corrompue (erreur sans code de l'OS, troncature, CRC) `archive_unreadable` ;
    /// même code de sortie (3).
    #[test]
    fn erreur_du_systeme_distincte_d_une_archive_corrompue() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/vectors/archive/deflate_metadata_ok.zip"
        );
        let data = std::fs::read(path).unwrap();
        type Make = fn() -> std::io::Error;
        let cases: [(Make, &str); 4] = [
            // Accès refusé : `EACCES` (13) sous Unix, `ERROR_ACCESS_DENIED` (5) sous Windows.
            (
                || std::io::Error::from_raw_os_error(if cfg!(windows) { 5 } else { 13 }),
                "archive_io",
            ),
            // `EIO` (5) sous Unix, `ERROR_SHARING_VIOLATION` (32) sous Windows.
            (
                || std::io::Error::from_raw_os_error(if cfg!(windows) { 32 } else { 5 }),
                "archive_io",
            ),
            // Erreur sans code de l'OS : construite par une couche de lecture, contenu en cause.
            (
                || std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "flux tronque"),
                "archive_unreadable",
            ),
            // Positionnement refusé (`InvalidInput`) : décalage absurde, contenu en cause.
            (
                || std::io::Error::from(std::io::ErrorKind::InvalidInput),
                "archive_unreadable",
            ),
        ];
        for (i, (error, code)) in cases.into_iter().enumerate() {
            let fail = std::rc::Rc::new(std::cell::Cell::new(false));
            let reader = Failing {
                inner: std::io::Cursor::new(data.clone()),
                fail: fail.clone(),
                error,
            };
            let mut archive = Archive::new(reader, "a.zip".into()).unwrap();
            let name = archive.names()[0].clone();
            fail.set(true);
            let e = archive.read(&name).unwrap_err();
            assert_eq!((e.code, e.exit_code), (code, 3), "cas {i} : {e}");
            let e = archive.read_cancellable(&name, &|| false).unwrap_err();
            assert_eq!((e.code, e.exit_code), (code, 3), "cas {i} : {e}");
        }
        // Archive tronquée : contenu en cause.
        let cut = data[..data.len() / 2].to_vec();
        let e = Archive::new(std::io::Cursor::new(cut), "a.zip".into())
            .err()
            .unwrap();
        assert_eq!(e.code, "archive_unreadable");
    }

    /// #580 : `read_cancellable` lit comme `read`, et s'arrête sur l'annulation.
    #[test]
    fn lecture_annulable_comme_la_lecture() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/vectors/archive/bzip2_multiblock.zip"
        );
        let data = std::fs::read(path).unwrap();
        let mut archive = Archive::new(std::io::Cursor::new(data), "a.zip".into()).unwrap();
        let name = "images/c.png";
        let want = archive.read(name).unwrap().unwrap();
        assert_eq!(
            archive.read_cancellable(name, &|| false).unwrap().unwrap(),
            want
        );
        assert_eq!(archive.read_cancellable("absent", &|| false).unwrap(), None);
        let e = archive.read_cancellable(name, &|| true).unwrap_err();
        assert_eq!((e.code, e.exit_code), ("cancelled", 130));
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
        // Le nom est cité comme le `%r` de Python.
        let e = check_asset_name("../x.png", "test").unwrap_err();
        assert!(e.message.ends_with(" : '../x.png'"), "{}", e.message);
        let e = check_asset_name("it's/../\u{1}", "test").unwrap_err();
        assert!(e.message.ends_with(" : \"it's/../\\x01\""), "{}", e.message);
        for ok in ["a.png", "img/a.png", "été/b.mp3", "..a/b", "a..png"] {
            assert_eq!(check_asset_name(ok, "test").unwrap(), ok);
        }
    }
}
