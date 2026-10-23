//! Écriture de la sortie : `.plain.pk` (ZIP *stored*) et dossier `.plain`, la fin de `_convert`
//! (`tools/telmi2flam/telmi2flam.py`) avec `_commit`, `_clean_orphans` et `_discard`.
//!
//! Invariants (PORT-RUST.md §3) :
//! - **jamais de sortie partielle** : tout s'écrit dans `<cible>.<pid>.part`, voisin de la
//!   cible, puis le `.part` est publié par renommage (`std::fs::rename`, soit
//!   `MoveFileExW(REPLACE_EXISTING)` ou son équivalent POSIX sous Windows), relancé quelques
//!   fois si le fichier est verrouillé (antivirus). Le `.plain.pk` est vidé sur disque
//!   (`sync_all`) avant le renommage : une coupure de courant ne publie pas un fichier creux ;
//! - **un échec ne détruit rien** : à chaque étape (création, écriture, `sync`, renommage,
//!   annulation), les `.part` sont supprimés et l'ancienne sortie reste intacte ;
//! - **aucune écriture hors de la destination** : garde zip-slip de Python sur les noms
//!   ([`check_names`]), et chaque dossier du `.plain` est vérifié par chemin canonique ;
//! - **octets du ZIP identiques à `zipfile.writestr`** (Python 3.14 sous Windows) à horodatage
//!   égal : méthode *stored*, versions 20 (45 en ZIP64), système créateur 0 (MS-DOS),
//!   attributs externes `0o600 << 16`, bit 11 (UTF-8) seulement pour un nom non ASCII, ZIP64
//!   aux mêmes seuils que `zipfile`. Seul écart : l'horodatage est fixé au 1980-01-01 00:00
//!   (Python prend l'heure locale, PORT-RUST.md §4.1) ; le lecteur du player
//!   (`src/formats/pk_reader.c`) l'ignore ;
//! - **MP3 référencés** ([`Data::Source`], #355) : relus dans l'archive d'entrée un à un, au
//!   moment de leur écriture (`.plain.pk` puis dossier `.plain`), et refusés
//!   (`archive_unreadable`, sortie non publiée) si leur taille ou leur CRC-32 diffère de ceux
//!   de l'assemblage. Une seule de ces entrées est en mémoire à la fois.
//!
//! Limites reprises de Python : le dossier `.plain` existant est mis à jour fichier par fichier
//! (`_commit`), donc sans atomicité, et les publications du `.plain.pk` puis du `.plain` ne sont
//! pas atomiques ensemble. Le launcher n'utilise que le `.plain.pk`.
//!
//! Écarts voulus avec Python : un `.part` laissé par un processus mort qui portait **notre**
//! PID est supprimé avant l'écriture (Python le réutiliserait et publierait ses fichiers
//! périmés dans le `.plain`) ; un `.plain` à publier sur un fichier existant (et non un
//! dossier) est refusé sans rien remplacer ; un PID orphelin n'est reconnu qu'en chiffres
//! ASCII (Python accepte tout `str.isdigit()`) ; un lien symbolique orphelin est supprimé sans
//! être suivi.

use std::borrow::Cow;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::Duration;

use crate::archive::Archive;
use crate::error::ConvError;
use crate::package::{Data, Package};
use crate::progress::{Phase, Progress};

/// `zipfile.ZIP64_LIMIT` : au-delà, tailles et positions passent en ZIP64.
pub const ZIP64_LIMIT: u64 = (1 << 31) - 1;
/// `zipfile.ZIP_FILECOUNT_LIMIT` : au-delà, le répertoire central passe en ZIP64.
pub const ZIP_FILECOUNT_LIMIT: usize = (1 << 16) - 1;

/// Seuils ZIP64 de `zipfile` ; [`ZipLimits::PYTHON`] en production, réduits par les tests pour
/// exercer les chemins ZIP64 sans écrire 2 Gio (le générateur de vecteurs réduit ceux de
/// Python de la même façon).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipLimits {
    pub zip64: u64,
    pub file_count: usize,
}

impl ZipLimits {
    pub const PYTHON: ZipLimits = ZipLimits {
        zip64: ZIP64_LIMIT,
        file_count: ZIP_FILECOUNT_LIMIT,
    };
}

/// Date DOS fixe des entrées : 1980-01-01 (`(0 << 9) | (1 << 5) | 1`).
const DOS_DATE: u16 = 0x21;
/// Heure DOS fixe des entrées : 00:00:00.
const DOS_TIME: u16 = 0;
/// `zipfile.DEFAULT_VERSION` et `zipfile.ZIP64_VERSION`.
const DEFAULT_VERSION: u8 = 20;
const ZIP64_VERSION: u8 = 45;
/// `ZipInfo.create_system` sous Windows (Python le met à 3 ailleurs).
const CREATE_SYSTEM: u8 = 0;
/// `writestr` : `external_attr = 0o600 << 16` (`?rw-------`).
const EXTERNAL_ATTR: u32 = 0o600 << 16;
/// Bit 11 : nom encodé en UTF-8.
const FLAG_UTF8: u16 = 0x800;

/// Garde zip-slip de `_convert` : noms relatifs, sans `\`, sans `:`, sans `..`, sans composant
/// vide. Refus : `invalid_content` (code 4), comme le `ConvError` de Python.
pub fn check_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<(), ConvError> {
    for name in names {
        if name.starts_with('/')
            || name.contains('\\')
            || name.contains(':')
            || name.split('/').any(|p| p.is_empty() || p == "..")
        {
            return Err(ConvError::invalid_content(format!(
                "nom de sortie refuse : {name:?}"
            )));
        }
    }
    Ok(())
}

/// Arrêt de l'écriture du ZIP : erreur d'entrée-sortie ou annulation.
#[derive(Debug)]
pub enum ZipStop {
    Io(io::Error),
    Cancelled,
    /// Entrée référencée illisible ou modifiée dans l'archive d'entrée.
    Source(ConvError),
}

/// Archive d'où relire les entrées [`Data::Source`].
pub(crate) trait Source {
    /// Octets de l'entrée `name`, refusés si leur taille ou leur CRC-32 diffère.
    fn fetch(&mut self, name: &str, size: u64, crc: u32) -> Result<Vec<u8>, ConvError>;
}

/// Pas d'archive : toute entrée référencée est refusée.
struct NoSource;

impl Source for NoSource {
    fn fetch(&mut self, name: &str, _: u64, _: u32) -> Result<Vec<u8>, ConvError> {
        Err(ConvError::internal_error(format!(
            "{name} : entree referencee sans archive"
        )))
    }
}

impl<R: Read + Seek> Source for Archive<R> {
    fn fetch(&mut self, name: &str, size: u64, crc: u32) -> Result<Vec<u8>, ConvError> {
        match self.read(name)? {
            Some(data) if data.len() as u64 == size && crc32fast::hash(&data) == crc => Ok(data),
            _ => Err(ConvError::archive_unreadable(format!(
                "{name} : entree modifiee ou disparue depuis sa lecture"
            ))),
        }
    }
}

/// Contenu d'une entrée à écrire : en mémoire, ou relu dans l'archive d'entrée.
pub(crate) trait Body {
    fn load(&self, src: &mut dyn Source) -> Result<Cow<'_, [u8]>, ConvError>;
}

impl Body for Vec<u8> {
    fn load(&self, _: &mut dyn Source) -> Result<Cow<'_, [u8]>, ConvError> {
        Ok(Cow::Borrowed(self))
    }
}

impl Body for Data {
    fn load(&self, src: &mut dyn Source) -> Result<Cow<'_, [u8]>, ConvError> {
        match self {
            Data::Bytes(b) => Ok(Cow::Borrowed(b)),
            Data::Source { name, size, crc } => Ok(Cow::Owned(src.fetch(name, *size, *crc)?)),
        }
    }
}

impl From<io::Error> for ZipStop {
    fn from(e: io::Error) -> Self {
        ZipStop::Io(e)
    }
}

/// Écrivain qui compte sa position (`fp.tell()` de Python) sans exiger `Seek`.
struct Counted<'a, W: Write> {
    inner: &'a mut W,
    pos: u64,
}

impl<W: Write> Counted<'_, W> {
    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.inner.write_all(bytes)?;
        self.pos += bytes.len() as u64;
        Ok(())
    }
}

/// Entrée écrite, pour le répertoire central.
struct Written<'a> {
    name: &'a [u8],
    flags: u16,
    crc: u32,
    size: u64,
    offset: u64,
    /// En-tête local en ZIP64 (`file_size * 1.05 > ZIP64_LIMIT`) : versions passées à 45.
    local_zip64: bool,
}

/// Nom encodé et drapeaux (`ZipInfo._encodeFilenameFlags`).
fn name_flags(name: &str) -> (&[u8], u16) {
    (name.as_bytes(), if name.is_ascii() { 0 } else { FLAG_UTF8 })
}

/// Écrit le `.plain.pk` : les entrées dans l'ordre donné, octets de `zipfile.ZipFile(path, "w",
/// ZIP_STORED)` puis `writestr(nom, données)` pour chacune, horodatage fixe. `cancelled` est
/// consulté avant chaque entrée ; `hook(i)` est appelé au milieu de l'entrée `i` (après son
/// en-tête), pour l'injection de panne.
pub fn write_zip<W: Write>(
    out: &mut W,
    entries: &[(String, Vec<u8>)],
    limits: ZipLimits,
    cancelled: &dyn Fn() -> bool,
    hook: &mut dyn FnMut(usize) -> io::Result<()>,
) -> Result<(), ZipStop> {
    zip_entries(out, entries, &mut NoSource, limits, cancelled, hook)
}

/// [`write_zip`] pour des entrées en mémoire ou relues dans `src` (une à la fois, après la
/// consultation de `cancelled` et avant l'en-tête local).
fn zip_entries<W: Write, D: Body>(
    out: &mut W,
    entries: &[(String, D)],
    src: &mut dyn Source,
    limits: ZipLimits,
    cancelled: &dyn Fn() -> bool,
    hook: &mut dyn FnMut(usize) -> io::Result<()>,
) -> Result<(), ZipStop> {
    let mut w = Counted { inner: out, pos: 0 };
    let mut written = Vec::with_capacity(entries.len());
    for (i, (name, body)) in entries.iter().enumerate() {
        if cancelled() {
            return Err(ZipStop::Cancelled);
        }
        let (name, flags) = name_flags(name);
        // Longueur du nom sur 16 bits : Python lève `struct.error` (rien n'est publié) ; ici,
        // `output_error` au lieu d'un ZIP dont la longueur de nom serait tronquée.
        if name.len() > 0xFFFF {
            return Err(ZipStop::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("nom d'entree trop long ({} octets)", name.len()),
            )));
        }
        let data = body.load(src).map_err(ZipStop::Source)?;
        let data: &[u8] = &data;
        let size = data.len() as u64;
        // `_open_to_write` : `zip64 = file_size * 1.05 > ZIP64_LIMIT` (calcul en flottant).
        let local_zip64 = (size as f64) * 1.05 > limits.zip64 as f64;
        let mut extra = Vec::new();
        let (mut csize32, mut size32) = (size as u32, size as u32);
        let version = if local_zip64 {
            extra.extend_from_slice(&1u16.to_le_bytes());
            extra.extend_from_slice(&16u16.to_le_bytes());
            extra.extend_from_slice(&size.to_le_bytes());
            extra.extend_from_slice(&size.to_le_bytes());
            csize32 = u32::MAX;
            size32 = u32::MAX;
            ZIP64_VERSION
        } else {
            DEFAULT_VERSION
        };
        let crc = crc32fast::hash(data);
        let offset = w.pos;
        let mut h = Vec::with_capacity(30 + name.len() + extra.len());
        h.extend_from_slice(b"PK\x03\x04");
        h.push(version);
        h.push(0); // reserved
        h.extend_from_slice(&flags.to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes()); // ZIP_STORED
        h.extend_from_slice(&DOS_TIME.to_le_bytes());
        h.extend_from_slice(&DOS_DATE.to_le_bytes());
        h.extend_from_slice(&crc.to_le_bytes());
        h.extend_from_slice(&csize32.to_le_bytes());
        h.extend_from_slice(&size32.to_le_bytes());
        h.extend_from_slice(&(name.len() as u16).to_le_bytes());
        h.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        h.extend_from_slice(name);
        h.extend_from_slice(&extra);
        w.put(&h)?;
        hook(i)?;
        w.put(data)?;
        written.push(Written {
            name,
            flags,
            crc,
            size,
            offset,
            local_zip64,
        });
    }

    // `_write_end_record`.
    let start_dir = w.pos;
    for e in &written {
        let mut extra: Vec<u64> = Vec::new();
        let (mut csize32, mut size32) = (e.size as u32, e.size as u32);
        if e.size > limits.zip64 {
            extra.extend([e.size, e.size]);
            csize32 = u32::MAX;
            size32 = u32::MAX;
        }
        let mut offset32 = e.offset as u32;
        if e.offset > limits.zip64 {
            extra.push(e.offset);
            offset32 = u32::MAX;
        }
        let mut extra_data = Vec::new();
        let mut version = if e.local_zip64 {
            ZIP64_VERSION
        } else {
            DEFAULT_VERSION
        };
        if !extra.is_empty() {
            extra_data.extend_from_slice(&1u16.to_le_bytes());
            extra_data.extend_from_slice(&(8 * extra.len() as u16).to_le_bytes());
            for v in &extra {
                extra_data.extend_from_slice(&v.to_le_bytes());
            }
            version = ZIP64_VERSION;
        }
        let mut c = Vec::with_capacity(46 + e.name.len() + extra_data.len());
        c.extend_from_slice(b"PK\x01\x02");
        c.push(version); // create_version
        c.push(CREATE_SYSTEM);
        c.push(version); // extract_version
        c.push(0); // reserved
        c.extend_from_slice(&e.flags.to_le_bytes());
        c.extend_from_slice(&0u16.to_le_bytes()); // ZIP_STORED
        c.extend_from_slice(&DOS_TIME.to_le_bytes());
        c.extend_from_slice(&DOS_DATE.to_le_bytes());
        c.extend_from_slice(&e.crc.to_le_bytes());
        c.extend_from_slice(&csize32.to_le_bytes());
        c.extend_from_slice(&size32.to_le_bytes());
        c.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
        c.extend_from_slice(&(extra_data.len() as u16).to_le_bytes());
        c.extend_from_slice(&0u16.to_le_bytes()); // commentaire
        c.extend_from_slice(&0u16.to_le_bytes()); // disque
        c.extend_from_slice(&0u16.to_le_bytes()); // internal_attr
        c.extend_from_slice(&EXTERNAL_ATTR.to_le_bytes());
        c.extend_from_slice(&offset32.to_le_bytes());
        c.extend_from_slice(e.name);
        c.extend_from_slice(&extra_data);
        w.put(&c)?;
    }
    let pos2 = w.pos;
    let mut count = written.len() as u64;
    let mut dir_size = pos2 - start_dir;
    let mut dir_offset = start_dir;
    if written.len() > limits.file_count || dir_offset > limits.zip64 || dir_size > limits.zip64 {
        let mut z = Vec::with_capacity(56 + 20);
        z.extend_from_slice(b"PK\x06\x06");
        z.extend_from_slice(&44u64.to_le_bytes());
        z.extend_from_slice(&45u16.to_le_bytes());
        z.extend_from_slice(&45u16.to_le_bytes());
        z.extend_from_slice(&0u32.to_le_bytes());
        z.extend_from_slice(&0u32.to_le_bytes());
        z.extend_from_slice(&count.to_le_bytes());
        z.extend_from_slice(&count.to_le_bytes());
        z.extend_from_slice(&dir_size.to_le_bytes());
        z.extend_from_slice(&dir_offset.to_le_bytes());
        z.extend_from_slice(b"PK\x06\x07");
        z.extend_from_slice(&0u32.to_le_bytes());
        z.extend_from_slice(&pos2.to_le_bytes());
        z.extend_from_slice(&1u32.to_le_bytes());
        w.put(&z)?;
        count = count.min(0xFFFF);
        dir_size = dir_size.min(0xFFFF_FFFF);
        dir_offset = dir_offset.min(0xFFFF_FFFF);
    }
    let mut end = Vec::with_capacity(22);
    end.extend_from_slice(b"PK\x05\x06");
    end.extend_from_slice(&0u16.to_le_bytes());
    end.extend_from_slice(&0u16.to_le_bytes());
    end.extend_from_slice(&(count as u16).to_le_bytes());
    end.extend_from_slice(&(count as u16).to_le_bytes());
    end.extend_from_slice(&(dir_size as u32).to_le_bytes());
    end.extend_from_slice(&(dir_offset as u32).to_le_bytes());
    end.extend_from_slice(&0u16.to_le_bytes()); // commentaire
    w.put(&end)?;
    Ok(())
}

/// Destination d'une conversion (`-o`, `--plain`), résolue comme `_convert`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Chemin de sortie (`-o`, ou nom par défaut dans le dossier courant).
    pub out_path: PathBuf,
    /// `-o` se termine par `.plain` : seul le dossier est écrit.
    pub dir_only: bool,
    /// Dossier `.plain` à écrire (`--plain` ou `dir_only`).
    pub plain_dir: Option<PathBuf>,
}

/// Nom de sortie par défaut : titre réduit à `[A-Za-z0-9_-]` (suites remplacées par `_`, `_`
/// retirés aux bords, `story` si vide), puis les 4 premiers octets de l'UUID en hexadécimal
/// majuscule : `<titre>.<UUID>.plain.pk`.
pub fn default_name(title: &str, uuid: &[u8; 16]) -> String {
    let mut safe = String::with_capacity(title.len());
    let mut in_run = false;
    for c in title.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            safe.push(c);
            in_run = false;
        } else if !in_run {
            safe.push('_');
            in_run = true;
        }
    }
    let safe = safe.trim_matches('_');
    let safe = if safe.is_empty() { "story" } else { safe };
    let hex: String = uuid[..4].iter().map(|b| format!("{b:02X}")).collect();
    format!("{safe}.{hex}.plain.pk")
}

/// Le chemin, tel que donné, se termine-t-il par `suffix` (ASCII) ? (`str.endswith`)
fn ends_with(path: &Path, suffix: &str) -> bool {
    path.as_os_str()
        .as_encoded_bytes()
        .ends_with(suffix.as_bytes())
}

/// `path` suivi de `suffix`, sur la chaîne (pas un composant ajouté).
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s: OsString = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

impl Target {
    /// `out` : option `-o` ; `emit_plain` : option `--plain`.
    pub fn new<D>(out: Option<&Path>, emit_plain: bool, package: &Package<D>) -> Target {
        let out_path = match out {
            Some(p) => p.to_owned(),
            None => PathBuf::from(default_name(&package.title, &package.uuid)),
        };
        let dir_only = ends_with(&out_path, ".plain");
        let plain_dir = if dir_only {
            Some(out_path.clone())
        } else if !emit_plain {
            None
        } else if ends_with(&out_path, ".plain.pk") {
            // `out_path[:-3]` : `with_extension("")` retire exactement `.pk`.
            Some(out_path.with_extension(""))
        } else {
            Some(with_suffix(&out_path, ".plain"))
        };
        Target {
            out_path,
            dir_only,
            plain_dir,
        }
    }

    /// Chemins publiés, dans l'ordre (`OK -> ...` de Python).
    pub fn published(&self) -> Vec<PathBuf> {
        let mut v = Vec::new();
        if !self.dir_only {
            v.push(self.out_path.clone());
        }
        v.extend(self.plain_dir.clone());
        v
    }
}

/// Temporaire voisin : `<cible>.<pid>.part`.
pub fn part_path(target: &Path) -> PathBuf {
    with_suffix(target, &format!(".{}.part", std::process::id()))
}

/// Étapes où un test injecte une panne.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// Création du `.part` du `.plain.pk`.
    CreatePk,
    /// Au milieu de l'entrée `i` du `.plain.pk` (en-tête écrit, données non).
    WritePk(usize),
    /// `sync_all` du `.plain.pk`.
    SyncPk,
    /// Création du dossier `.part` du `.plain`.
    CreateDir,
    /// Fichier `i` du dossier `.plain`.
    WriteDir(usize),
    /// Tentative `attempt` (0 = première) d'un renommage de publication.
    Rename(u32),
}

type Hook<'a> = &'a mut dyn FnMut(Step) -> io::Result<()>;

/// Écrit le paquet vers `target` et renvoie les chemins publiés. Ordre de Python : garde
/// zip-slip, nettoyage des orphelins, `.plain.pk` dans son `.part`, dossier `.plain` dans le
/// sien, puis les publications. Erreur d'entrée-sortie : `output_error` (code 6), sans `.part`
/// restant ; annulation (consultée avant chaque entrée et avant les publications) :
/// `cancelled`.
pub fn write_package(
    package: &Package,
    target: &Target,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<PathBuf>, ConvError> {
    write_package_progress(package, target, cancelled, &mut ())
}

/// [`write_package`] avec la phase `zip` : annoncée après la garde zip-slip, total = nombre
/// d'entrées × nombre de sorties (`.plain.pk`, dossier `.plain`), une étape par fichier écrit.
pub fn write_package_progress(
    package: &Package,
    target: &Target,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn Progress,
) -> Result<Vec<PathBuf>, ConvError> {
    write_package_with(
        package,
        target,
        ZipLimits::PYTHON,
        cancelled,
        progress,
        &mut |_| Ok(()),
    )
}

/// [`write_package_progress`] d'un paquet de [`crate::package::build_package_streamed`] : les
/// entrées [`Data::Source`] sont relues dans `archive` (celle de l'assemblage), une à la fois.
pub fn write_package_from<R: Read + Seek>(
    package: &Package<Data>,
    archive: &mut Archive<R>,
    target: &Target,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn Progress,
) -> Result<Vec<PathBuf>, ConvError> {
    write_any(
        package,
        archive,
        target,
        ZipLimits::PYTHON,
        cancelled,
        progress,
        &mut |_| Ok(()),
    )
}

pub(crate) fn write_package_with(
    package: &Package,
    target: &Target,
    limits: ZipLimits,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn Progress,
    hook: Hook<'_>,
) -> Result<Vec<PathBuf>, ConvError> {
    write_any(
        package,
        &mut NoSource,
        target,
        limits,
        cancelled,
        progress,
        hook,
    )
}

pub(crate) fn write_any<D: Body>(
    package: &Package<D>,
    src: &mut dyn Source,
    target: &Target,
    limits: ZipLimits,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn Progress,
    hook: Hook<'_>,
) -> Result<Vec<PathBuf>, ConvError> {
    check_names(package.entries.iter().map(|(n, _)| n.as_str()))?;
    let outputs = u64::from(!target.dir_only) + u64::from(target.plain_dir.is_some());
    progress.begin(Phase::Zip, package.entries.len() as u64 * outputs);
    clean_orphans(&target.out_path);
    if let Some(dir) = &target.plain_dir {
        if dir != &target.out_path {
            clean_orphans(dir);
        }
    }
    let mut tmps = Vec::new();
    let result = write_and_commit(
        package, src, target, limits, cancelled, progress, hook, &mut tmps,
    );
    discard(&tmps);
    untrack(&tmps);
    match result {
        Ok(()) => Ok(target.published()),
        Err(Fail::Conv(e)) => Err(e),
        Err(Fail::Io(e)) => Err(ConvError::output_error(format!(
            "ecriture de {} impossible : {e}",
            target.out_path.display()
        ))),
    }
}

enum Fail {
    Io(io::Error),
    Conv(ConvError),
}

impl From<io::Error> for Fail {
    fn from(e: io::Error) -> Self {
        Fail::Io(e)
    }
}

impl From<ZipStop> for Fail {
    fn from(e: ZipStop) -> Self {
        match e {
            ZipStop::Io(e) => Fail::Io(e),
            ZipStop::Cancelled => Fail::Conv(ConvError::cancelled()),
            ZipStop::Source(e) => Fail::Conv(e),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn write_and_commit<D: Body>(
    package: &Package<D>,
    src: &mut dyn Source,
    target: &Target,
    limits: ZipLimits,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn Progress,
    hook: Hook<'_>,
    tmps: &mut Vec<PathBuf>,
) -> Result<(), Fail> {
    let mut commits = Vec::new();
    if !target.dir_only {
        let abs = std::path::absolute(&target.out_path)?;
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = part_path(&target.out_path);
        discard(std::slice::from_ref(&tmp));
        track(&tmp);
        tmps.push(tmp.clone());
        hook(Step::CreatePk)?;
        let file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        let mut buf = BufWriter::with_capacity(1 << 20, file);
        // Une étape par entrée écrite (après `writestr` en Python) : l'entrée `i - 1` est
        // complète quand l'en-tête de l'entrée `i` est écrit, la dernière au retour.
        zip_entries(
            &mut buf,
            &package.entries,
            src,
            limits,
            cancelled,
            &mut |i| {
                if i > 0 {
                    progress.tick(Phase::Zip);
                }
                hook(Step::WritePk(i))
            },
        )?;
        if !package.entries.is_empty() {
            progress.tick(Phase::Zip);
        }
        let file = buf.into_inner().map_err(|e| e.into_error())?;
        hook(Step::SyncPk)?;
        file.sync_all()?;
        drop(file);
        commits.push((tmp, target.out_path.clone()));
    }
    if let Some(plain_dir) = &target.plain_dir {
        let tmp = part_path(plain_dir);
        discard(std::slice::from_ref(&tmp));
        track(&tmp);
        tmps.push(tmp.clone());
        hook(Step::CreateDir)?;
        if let Some(parent) = std::path::absolute(&tmp)?.parent() {
            fs::create_dir_all(parent)?;
        }
        // `create_dir` et non `create_dir_all` : un lien ou une jonction placé à ce nom entre
        // `discard` et ici est refusé au lieu d'être suivi (écriture hors de la destination).
        fs::create_dir(&tmp)?;
        let real = fs::canonicalize(&tmp)?;
        for (i, (name, body)) in package.entries.iter().enumerate() {
            if cancelled() {
                return Err(Fail::Conv(ConvError::cancelled()));
            }
            let data = body.load(src).map_err(Fail::Conv)?;
            let mut parts: Vec<&str> = name.split('/').collect();
            let file_name = parts.pop().unwrap_or_default();
            let parent = parts.iter().fold(real.clone(), |p, c| p.join(c));
            fs::create_dir_all(&parent)?;
            if !fs::canonicalize(&parent)?.starts_with(&real) {
                return Err(Fail::Conv(ConvError::invalid_content(format!(
                    "ecriture hors du dossier .plain refusee : {name:?}"
                ))));
            }
            hook(Step::WriteDir(i))?;
            // `create_new` : jamais d'écriture à travers un lien déjà présent.
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(parent.join(file_name))?;
            f.write_all(&data)?;
            progress.tick(Phase::Zip);
        }
        commits.push((tmp, plain_dir.clone()));
    }
    if cancelled() {
        return Err(Fail::Conv(ConvError::cancelled()));
    }
    for (tmp, fin) in &commits {
        commit(tmp, fin, hook)?;
    }
    Ok(())
}

/// `_commit` : publie un temporaire. Dossier vers dossier existant : fichiers remplacés un à
/// un sans supprimer le reste (comportement historique), puis temporaire supprimé.
fn commit(tmp: &Path, fin: &Path, hook: Hook<'_>) -> io::Result<()> {
    let tmp_is_dir = fs::symlink_metadata(tmp)?.is_dir();
    let fin_meta = fs::metadata(fin).ok();
    match (tmp_is_dir, fin_meta) {
        (true, Some(m)) if m.is_dir() => {
            merge_dir(tmp, fin, hook)?;
            let _ = fs::remove_dir_all(tmp);
            Ok(())
        }
        // Python échoue aussi (`os.replace` d'un dossier sur un fichier, ou l'inverse) ; le
        // refus explicite évite qu'un renommage POSIX remplace quand même la cible.
        (true, Some(_)) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} existe et n'est pas un dossier", fin.display()),
        )),
        (false, Some(m)) if m.is_dir() => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} est un dossier", fin.display()),
        )),
        _ => rename_retry(tmp, fin, hook),
    }
}

/// `os.walk(tmp)` puis `os.replace` de chaque fichier vers le dossier final.
fn merge_dir(tmp: &Path, fin: &Path, hook: Hook<'_>) -> io::Result<()> {
    fs::create_dir_all(fin)?;
    let mut entries: Vec<_> = fs::read_dir(tmp)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let dest = fin.join(e.file_name());
        if e.file_type()?.is_dir() {
            merge_dir(&e.path(), &dest, hook)?;
        } else {
            rename_retry(&e.path(), &dest, hook)?;
        }
    }
    Ok(())
}

/// Délais entre les tentatives de renommage (fichier verrouillé par un antivirus).
const RENAME_RETRY_DELAYS_MS: [u64; 5] = [50, 100, 200, 400, 800];

/// Erreur transitoire : accès refusé, violation de partage ou de verrou (Windows).
#[cfg(windows)]
fn is_transient(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(5 | 32 | 33))
}

/// Hors Windows, un renommage n'est pas bloqué par un verrou : aucune relance.
#[cfg(not(windows))]
fn is_transient(_e: &io::Error) -> bool {
    false
}

fn rename_retry(from: &Path, to: &Path, hook: Hook<'_>) -> io::Result<()> {
    let mut attempt = 0u32;
    loop {
        let r = hook(Step::Rename(attempt)).and_then(|()| fs::rename(from, to));
        match r {
            Err(e) if is_transient(&e) && (attempt as usize) < RENAME_RETRY_DELAYS_MS.len() => {
                if !cfg!(test) {
                    std::thread::sleep(Duration::from_millis(
                        RENAME_RETRY_DELAYS_MS[attempt as usize],
                    ));
                }
                attempt += 1;
            }
            r => return r,
        }
    }
}

/// Registre global des temporaires en cours (`.part` de toutes les écritures du processus), lu
/// par [`discard_pending`] : avec `panic = "abort"`, aucun `Drop` ne s'exécute après le crochet
/// de panique, qui doit donc retrouver lui-même les chemins à supprimer.
static PENDING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Verrou du registre, même empoisonné (le registre n'est jamais laissé incohérent : un `push`
/// ou un `remove` ne panique pas à mi-chemin).
fn pending() -> MutexGuard<'static, Vec<PathBuf>> {
    PENDING.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Inscrit `tmp` **avant** sa création : une panique entre la création et le nettoyage le
/// retrouve toujours. Fichier ou dossier : tout temporaire de la crate peut s'y inscrire (par
/// exemple le dossier de travail de ffmpeg), puis en être retiré par [`untrack`].
pub(crate) fn track(tmp: &Path) {
    pending().push(tmp.to_path_buf());
}

/// Retire `tmps` du registre **après** leur suppression ou leur publication. Retrait par
/// valeur, une occurrence chacun : des écritures concurrentes (bibliothèque) gardent les leurs.
pub(crate) fn untrack(tmps: &[PathBuf]) {
    let mut reg = pending();
    for t in tmps {
        if let Some(i) = reg.iter().rposition(|p| p == t) {
            reg.swap_remove(i);
        }
    }
}

/// Supprime les temporaires inscrits au registre (crochet de panique du binaire). Le verrou est
/// tenté pendant environ 1 s au plus, sans bloquer : si le fil qui panique le détient déjà
/// (impossible en pratique, voir [`pending`]), rien n'est supprimé plutôt que de se bloquer.
/// Un chemin déjà publié (renommé) ou supprimé ne coûte rien. Les entrées restent inscrites :
/// le processus s'arrête ensuite.
pub fn discard_pending() {
    for _ in 0..100 {
        match PENDING.try_lock() {
            Ok(reg) => return discard(&reg),
            Err(TryLockError::Poisoned(p)) => return discard(&p.into_inner()),
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// `_discard` : supprime fichiers et dossiers, sans erreur. Un lien symbolique (ou une
/// jonction) est supprimé sans être suivi.
pub fn discard(paths: &[PathBuf]) {
    for p in paths {
        let Ok(meta) = fs::symlink_metadata(p) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            if fs::remove_file(p).is_err() {
                let _ = fs::remove_dir(p);
            }
        } else if meta.is_dir() {
            // `remove_dir_all` ne suit pas les liens qu'il rencontre.
            let _ = fs::remove_dir_all(p);
        } else {
            let _ = fs::remove_file(p);
        }
    }
}

/// `_clean_orphans` : supprime les `<cible>.<pid>.part` voisins dont le PID n'est plus vivant
/// (une conversion concurrente vivante est épargnée). Arrêt brutal puis reprise : le `.part`
/// laissé disparaît à la conversion suivante vers la même cible. Un PID réutilisé empêche ce
/// nettoyage (limite connue).
pub fn clean_orphans(target: &Path) {
    let Ok(abs) = std::path::absolute(target) else {
        return;
    };
    let (Some(dir), Some(base)) = (abs.parent(), target.file_name()) else {
        return;
    };
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    let mut prefix = base.as_encoded_bytes().to_vec();
    prefix.push(b'.');
    for entry in read.flatten() {
        let name = entry.file_name();
        let bytes = name.as_encoded_bytes();
        let Some(pid) = bytes
            .strip_prefix(prefix.as_slice())
            .and_then(|r| r.strip_suffix(b".part"))
        else {
            continue;
        };
        if pid.is_empty() || !pid.iter().all(u8::is_ascii_digit) {
            continue;
        }
        // Au-delà de 32 bits, aucun processus ne peut porter ce PID ; Python échouerait
        // (`ctypes`), on préfère épargner le fichier.
        let Some(pid) = std::str::from_utf8(pid)
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        if !pid_alive(pid) {
            discard(&[dir.join(&name)]);
        }
    }
}

/// `_pid_alive` : le processus `pid` existe-t-il encore ?
pub fn pid_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    sys::pid_alive(pid)
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;

    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    const ERROR_ACCESS_DENIED: i32 = 5;

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn GetExitCodeProcess(process: *mut c_void, code: *mut u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    pub fn pid_alive(pid: u32) -> bool {
        // SAFETY : appels Win32 sans pointeur entrant ; le handle obtenu est fermé une fois.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                // Accès refusé : le processus existe.
                return std::io::Error::last_os_error().raw_os_error() == Some(ERROR_ACCESS_DENIED);
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(h, &mut code) != 0;
            CloseHandle(h);
            ok && code == STILL_ACTIVE
        }
    }
}

#[cfg(unix)]
mod sys {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }

    pub fn pid_alive(pid: u32) -> bool {
        let Ok(pid) = i32::try_from(pid) else {
            return true;
        };
        // SAFETY : signal 0, aucun effet ; seule l'existence du processus est testée.
        if unsafe { kill(pid, 0) } == 0 {
            return true;
        }
        // ESRCH (3) : pas de processus ; EPERM ou autre : il existe.
        std::io::Error::last_os_error().raw_os_error() != Some(3)
    }
}

#[cfg(not(any(windows, unix)))]
mod sys {
    /// Sans moyen de le savoir, le processus est supposé vivant (rien n'est supprimé).
    pub fn pid_alive(_pid: u32) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::Archive;
    use std::cell::{Cell, RefCell};
    use std::io::Cursor;

    /// Dossier de test vide, propre au processus et au test, supprimé à la fin.
    struct Scratch(PathBuf);

    impl std::ops::Deref for Scratch {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.0
        }
    }

    impl AsRef<Path> for Scratch {
        fn as_ref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let d = std::env::temp_dir().join(format!("t2f-output-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        Scratch(d)
    }

    fn package(entries: &[(&str, &[u8])]) -> Package {
        Package {
            entries: entries
                .iter()
                .map(|(n, d)| ((*n).to_owned(), d.to_vec()))
                .collect(),
            warnings: Vec::new(),
            title: "Titre".to_owned(),
            uuid: [0xAB; 16],
        }
    }

    fn sample() -> Package {
        package(&[
            ("info.plain", b"T\nT\n000000\nT"),
            ("version", b"1"),
            ("img/a.lif", &[7u8; 5000]),
            ("script/x/y.lua", b"return 1"),
            ("sounds/s.mp3", &[0xFFu8; 3000]),
            ("uuid.bin", &[0xAB; 16]),
        ])
    }

    /// Entrées du `.plain.pk` relues par le lecteur ZIP de la crate (CRC vérifiés).
    fn read_pk(path: &Path) -> Vec<(String, Vec<u8>)> {
        let bytes = fs::read(path).unwrap();
        let mut a = Archive::new(Cursor::new(bytes), "pk".into()).unwrap();
        let names = a.names().to_vec();
        names
            .into_iter()
            .map(|n| {
                let d = a.read(&n).unwrap().unwrap();
                (n, d)
            })
            .collect()
    }

    /// Aucun `.part` dans `dir` ni dessous.
    fn assert_no_part(dir: &Path) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            assert!(
                !name.ends_with(".part"),
                "{} : .part restant",
                e.path().display()
            );
            if e.file_type().unwrap().is_dir() {
                assert_no_part(&e.path());
            }
        }
    }

    fn never() -> bool {
        false
    }

    #[test]
    fn garde_zip_slip() {
        for bad in [
            "/abs", "a\\b", "c:x", "a/../b", "..", "a//b", "a/", "", "./x/..",
        ] {
            let e = check_names([bad]).unwrap_err();
            assert_eq!(e.code, "invalid_content", "{bad:?}");
        }
        check_names(["a", "a/b.c", "x/.y", "...", "é/ü"]).unwrap();
        // Rien n'est écrit, aucun `.part`.
        let d = scratch("zipslip");
        let target = Target::new(Some(&d.join("o.plain.pk")), true, &sample());
        let pkg = package(&[("ok", b"1"), ("a/../../evil", b"2")]);
        let e = write_package(&pkg, &target, &never).unwrap_err();
        assert_eq!(e.code, "invalid_content");
        assert_eq!(fs::read_dir(&d).unwrap().count(), 0);
    }

    #[test]
    fn nom_par_defaut_et_cibles() {
        let u = [0x0Au8, 0xBC, 0x01, 0xFF, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9];
        assert_eq!(
            default_name("Le Petit-Chat !", &u),
            "Le_Petit-Chat.0ABC01FF.plain.pk"
        );
        assert_eq!(default_name("Été à Paris", &u), "t_Paris.0ABC01FF.plain.pk");
        assert_eq!(default_name("__a__b__", &u), "a__b.0ABC01FF.plain.pk");
        assert_eq!(default_name("日本", &u), "story.0ABC01FF.plain.pk");
        assert_eq!(default_name("", &u), "story.0ABC01FF.plain.pk");

        let pkg = sample();
        let t = Target::new(None, false, &pkg);
        assert_eq!(t.out_path, PathBuf::from("Titre.ABABABAB.plain.pk"));
        assert_eq!((t.dir_only, t.plain_dir), (false, None));
        let t = Target::new(Some(Path::new("d/x.plain.pk")), true, &pkg);
        assert_eq!(t.plain_dir, Some(PathBuf::from("d/x.plain")));
        assert_eq!(
            t.published(),
            [PathBuf::from("d/x.plain.pk"), PathBuf::from("d/x.plain")]
        );
        let t = Target::new(Some(Path::new("x.zip")), true, &pkg);
        assert_eq!(t.plain_dir, Some(PathBuf::from("x.zip.plain")));
        let t = Target::new(Some(Path::new("é.plain")), false, &pkg);
        assert!(t.dir_only);
        assert_eq!(t.published(), [PathBuf::from("é.plain")]);
        let t = Target::new(Some(Path::new(".plain.pk")), true, &pkg);
        assert_eq!(t.plain_dir, Some(PathBuf::from(".plain")));
        assert_eq!(
            part_path(Path::new("a/b.plain.pk")),
            PathBuf::from(format!("a/b.plain.pk.{}.part", std::process::id()))
        );
    }

    #[test]
    fn ecrit_le_pk_et_le_dossier() {
        let d = scratch("ecrit");
        let pkg = sample();
        // Dossier parent créé à la demande.
        let target = Target::new(Some(&d.join("sous/o.plain.pk")), true, &pkg);
        let out = write_package(&pkg, &target, &never).unwrap();
        assert_eq!(out, target.published());
        assert_eq!(read_pk(&d.join("sous/o.plain.pk")), pkg.entries);
        for (name, data) in &pkg.entries {
            assert_eq!(&fs::read(d.join("sous/o.plain").join(name)).unwrap(), data);
        }
        assert_no_part(&d);
        // Deuxième conversion : le dossier existant est mis à jour sans perdre ses autres
        // fichiers (`_commit`), le `.plain.pk` est remplacé.
        fs::write(d.join("sous/o.plain/autre.txt"), b"garde").unwrap();
        let pkg2 = package(&[("version", b"2"), ("img/a.lif", b"neuf")]);
        write_package(&pkg2, &target, &never).unwrap();
        assert_eq!(read_pk(&d.join("sous/o.plain.pk")), pkg2.entries);
        assert_eq!(fs::read(d.join("sous/o.plain/version")).unwrap(), b"2");
        assert_eq!(fs::read(d.join("sous/o.plain/img/a.lif")).unwrap(), b"neuf");
        assert_eq!(
            fs::read(d.join("sous/o.plain/autre.txt")).unwrap(),
            b"garde"
        );
        assert_eq!(
            fs::read(d.join("sous/o.plain/script/x/y.lua")).unwrap(),
            b"return 1"
        );
        assert_no_part(&d);
        // `-o *.plain` : seulement le dossier.
        let t = Target::new(Some(&d.join("seul.plain")), false, &pkg);
        write_package(&pkg, &t, &never).unwrap();
        assert!(d.join("seul.plain/uuid.bin").is_file());
        assert!(!d.join("seul.plain.pk").exists());
        assert_no_part(&d);
    }

    /// État initial des pannes : une ancienne sortie complète (`.plain.pk` et `.plain`).
    fn old_output(d: &Path) {
        fs::write(d.join("o.plain.pk"), b"ancien paquet").unwrap();
        fs::create_dir_all(d.join("o.plain/img")).unwrap();
        fs::write(d.join("o.plain/version"), b"ancien").unwrap();
        fs::write(d.join("o.plain/img/a.lif"), b"ancien").unwrap();
    }

    /// Après une panne : chaque sortie est l'ancienne ou la nouvelle complète, jamais un
    /// fichier partiel ; aucun `.part`.
    fn assert_old_or_new(d: &Path, pkg: &Package, what: &str) {
        let pk = d.join("o.plain.pk");
        if fs::read(&pk).unwrap() != b"ancien paquet" {
            assert_eq!(read_pk(&pk), pkg.entries, "{what} : .plain.pk partiel");
        }
        for name in ["version", "img/a.lif"] {
            let got = fs::read(d.join("o.plain").join(name)).unwrap();
            let new = &pkg.entries.iter().find(|(n, _)| n == name).unwrap().1;
            assert!(got == b"ancien" || &got == new, "{what} : {name} partiel");
        }
        assert_no_part(d);
    }

    /// Panne injectée à chaque étape d'écriture, une à une : création, écriture de chaque
    /// entrée (au milieu de l'entrée, erreur quelconque ou disque plein), `sync`, création du
    /// dossier, chaque fichier du dossier, chaque renommage. Jamais de sortie partielle,
    /// ancienne sortie intacte tant que rien n'est publié, aucun `.part`, puis reprise.
    #[test]
    fn panne_a_chaque_etape() {
        let pkg = sample();
        let d = scratch("pannes");
        old_output(&d);
        let target = Target::new(Some(&d.join("o.plain.pk")), true, &pkg);
        let steps = RefCell::new(Vec::new());
        write_package_with(
            &pkg,
            &target,
            ZipLimits::PYTHON,
            &never,
            &mut (),
            &mut |s| {
                steps.borrow_mut().push(s);
                Ok(())
            },
        )
        .unwrap();
        let steps = steps.into_inner();
        let n = pkg.entries.len();
        let count = |f: fn(&Step) -> bool| steps.iter().filter(|s| f(s)).count();
        assert_eq!(steps.first(), Some(&Step::CreatePk));
        assert_eq!(count(|s| matches!(s, Step::WritePk(_))), n);
        assert_eq!(count(|s| matches!(s, Step::WriteDir(_))), n);
        assert!(steps.contains(&Step::SyncPk) && steps.contains(&Step::CreateDir));
        // Un renommage pour le `.plain.pk`, un par fichier du dossier (fusion).
        assert_eq!(count(|s| matches!(s, Step::Rename(_))), 1 + n);

        for (k, step) in steps.iter().enumerate() {
            for disk_full in [false, true] {
                let d = scratch("panne");
                old_output(&d);
                let target = Target::new(Some(&d.join("o.plain.pk")), true, &pkg);
                let seen = Cell::new(0usize);
                let err = write_package_with(
                    &pkg,
                    &target,
                    ZipLimits::PYTHON,
                    &never,
                    &mut (),
                    &mut |_| {
                        seen.set(seen.get() + 1);
                        if seen.get() != k + 1 {
                            Ok(())
                        } else if disk_full {
                            // ERROR_DISK_FULL sous Windows.
                            Err(io::Error::from_raw_os_error(112))
                        } else {
                            Err(io::Error::other("panne injectee"))
                        }
                    },
                )
                .unwrap_err();
                let what = format!("étape {k} ({step:?}, disque plein : {disk_full})");
                assert_eq!(err.code, "output_error", "{what}");
                assert_eq!(err.exit_code, 6);
                assert_old_or_new(&d, &pkg, &what);
                if !matches!(step, Step::Rename(_)) {
                    assert_eq!(fs::read(d.join("o.plain.pk")).unwrap(), b"ancien paquet");
                    assert_eq!(fs::read(d.join("o.plain/version")).unwrap(), b"ancien");
                }
                // Reprise : une nouvelle conversion publie la sortie complète.
                write_package(&pkg, &target, &never).unwrap();
                assert_eq!(read_pk(&d.join("o.plain.pk")), pkg.entries, "{what}");
                assert_no_part(&d);
            }
        }
    }

    /// Verrou transitoire (antivirus) : le renommage est relancé ; persistant, il échoue
    /// proprement après 6 tentatives, ancienne sortie intacte.
    #[test]
    fn renommage_relance() {
        let pkg = sample();
        for failures in [1u32, 5, 6] {
            let d = scratch("relance");
            old_output(&d);
            let target = Target::new(Some(&d.join("o.plain.pk")), false, &pkg);
            let attempts = Cell::new(0u32);
            let r = write_package_with(
                &pkg,
                &target,
                ZipLimits::PYTHON,
                &never,
                &mut (),
                &mut |s| {
                    if let Step::Rename(a) = s {
                        attempts.set(a + 1);
                        if a < failures {
                            // ERROR_SHARING_VIOLATION.
                            return Err(io::Error::from_raw_os_error(32));
                        }
                    }
                    Ok(())
                },
            );
            if failures <= 5 {
                r.unwrap();
                assert_eq!(attempts.get(), failures + 1);
                assert_eq!(read_pk(&d.join("o.plain.pk")), pkg.entries);
            } else {
                assert_eq!(r.unwrap_err().code, "output_error");
                assert_eq!(attempts.get(), 6);
                assert_eq!(fs::read(d.join("o.plain.pk")).unwrap(), b"ancien paquet");
            }
            assert_no_part(&d);
        }
    }

    fn tracked(p: &Path) -> bool {
        pending().iter().any(|q| q == p)
    }

    /// Registre des temporaires (#132) : chaque `.part` y est inscrit pendant l'écriture, puis
    /// retiré au retour, en succès comme en échec. Une panique en cours d'écriture (le crochet
    /// du binaire ne rend pas la main) laisse les `.part` inscrits, donc retrouvables.
    /// `discard_pending` n'est pas appelé ici : il viderait les `.part` des tests concurrents.
    #[test]
    fn registre_des_temporaires() {
        let pkg = sample();
        let d = scratch("registre");
        let target = Target::new(Some(&d.join("o.plain.pk")), true, &pkg);
        let pk_part = part_path(&target.out_path);
        let dir_part = part_path(target.plain_dir.as_ref().unwrap());
        // Pendant l'écriture : inscrits (vu depuis le crochet de chaque étape).
        let seen = Cell::new(false);
        write_package_with(
            &pkg,
            &target,
            ZipLimits::PYTHON,
            &never,
            &mut (),
            &mut |s| {
                if s == Step::CreateDir {
                    assert!(tracked(&pk_part) && tracked(&dir_part));
                    seen.set(true);
                }
                Ok(())
            },
        )
        .unwrap();
        assert!(seen.get());
        assert!(
            !tracked(&pk_part) && !tracked(&dir_part),
            "succès : retirés"
        );
        // Échec : retirés aussi.
        let r = write_package_with(
            &pkg,
            &target,
            ZipLimits::PYTHON,
            &never,
            &mut (),
            &mut |s| match s {
                Step::WriteDir(1) => Err(io::Error::other("panne")),
                _ => Ok(()),
            },
        );
        assert_eq!(r.unwrap_err().code, "output_error");
        assert!(!tracked(&pk_part) && !tracked(&dir_part), "échec : retirés");
        // Panique au milieu du `.plain.pk` : le `.part` existe et reste inscrit.
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = write_package_with(
                &pkg,
                &target,
                ZipLimits::PYTHON,
                &never,
                &mut (),
                &mut |s| {
                    if s == Step::WritePk(1) {
                        panic!("panique injectee");
                    }
                    Ok(())
                },
            );
        }));
        assert!(r.is_err());
        assert!(pk_part.exists() && tracked(&pk_part));
        discard(std::slice::from_ref(&pk_part));
        untrack(std::slice::from_ref(&pk_part));
        assert_no_part(&d);
    }

    /// Annulation avant chaque entrée (`.plain.pk` puis dossier) et avant les publications.
    #[test]
    fn annulation_a_chaque_entree() {
        let pkg = sample();
        let total = 2 * pkg.entries.len() + 1;
        for n in 0..total {
            let d = scratch("annule");
            old_output(&d);
            let target = Target::new(Some(&d.join("o.plain.pk")), true, &pkg);
            let calls = Cell::new(0usize);
            let cancel = || {
                calls.set(calls.get() + 1);
                calls.get() > n
            };
            let e = write_package(&pkg, &target, &cancel).unwrap_err();
            assert_eq!(e.code, "cancelled", "appel {n}");
            assert_eq!(fs::read(d.join("o.plain.pk")).unwrap(), b"ancien paquet");
            assert_eq!(fs::read(d.join("o.plain/version")).unwrap(), b"ancien");
            assert_no_part(&d);
        }
    }

    /// Processus fils tué et attendu à la fin, même si le test panique.
    struct Child(std::process::Child);

    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// Processus fils qui dure (pour un PID vivant).
    fn long_child() -> Child {
        let mut c = if cfg!(windows) {
            let mut c = std::process::Command::new("ping");
            c.args(["-n", "60", "127.0.0.1"]);
            c
        } else {
            let mut c = std::process::Command::new("sleep");
            c.arg("60");
            c
        };
        Child(c.stdout(std::process::Stdio::null()).spawn().unwrap())
    }

    /// Arrêt brutal puis reprise : les `.part` d'un PID mort (fichier tronqué, dossier) sont
    /// supprimés par la conversion suivante ; celui d'une conversion vivante est épargné, de
    /// même que les noms qui ne sont pas `<cible>.<chiffres>.part`.
    #[test]
    fn orphelins_apres_arret_brutal() {
        let pkg = sample();
        let d = scratch("orphelins");
        let dead = long_child();
        let dead_pid = dead.0.id();
        drop(dead);
        assert!(!pid_alive(dead_pid));
        let alive = long_child();
        let alive_pid = alive.0.id();
        assert!(pid_alive(alive_pid));
        assert!(pid_alive(std::process::id()));

        // Ce qu'un arrêt brutal laisse : `.part` tronqué, dossier `.part` à moitié écrit.
        fs::write(
            d.join(format!("o.plain.pk.{dead_pid}.part")),
            b"PK\x03\x04tronque",
        )
        .unwrap();
        fs::create_dir_all(d.join(format!("o.plain.{dead_pid}.part/img"))).unwrap();
        fs::write(d.join(format!("o.plain.{dead_pid}.part/img/a.lif")), b"x").unwrap();
        // Notre propre PID (processus mort qui portait le même numéro) : réécrit à neuf.
        fs::write(
            d.join(format!("o.plain.pk.{}.part", std::process::id())),
            b"vieux",
        )
        .unwrap();
        let kept = [
            format!("o.plain.pk.{alive_pid}.part"),
            "o.plain.pk.abc.part".to_owned(),
            "o.plain.pk..part".to_owned(),
            "o.plain.pk.99999999999.part".to_owned(),
            format!("autre.plain.pk.{dead_pid}.part"),
        ];
        for k in &kept {
            fs::write(d.join(k), b"garde").unwrap();
        }
        let pk = d.join("o.plain.pk");
        let target = Target::new(Some(&pk), true, &pkg);
        write_package(&pkg, &target, &never).unwrap();
        drop(alive);

        let mut left: Vec<String> = fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".part"))
            .collect();
        left.sort();
        let mut want = kept.to_vec();
        want.sort();
        assert_eq!(left, want);
        assert_eq!(read_pk(&pk), pkg.entries);
        // Le dossier publié ne contient pas les restes du `.part` mort (« x »).
        assert_eq!(fs::read(d.join("o.plain/img/a.lif")).unwrap(), [7u8; 5000]);
    }

    /// Cible d'un autre type : refus sans rien remplacer, aucun `.part`.
    #[test]
    fn cible_de_type_inattendu() {
        let pkg = sample();
        let d = scratch("type");
        fs::create_dir_all(d.join("o.plain.pk")).unwrap();
        fs::write(d.join("o.plain.pk/dedans"), b"x").unwrap();
        let t = Target::new(Some(&d.join("o.plain.pk")), false, &pkg);
        assert_eq!(
            write_package(&pkg, &t, &never).unwrap_err().code,
            "output_error"
        );
        assert_eq!(fs::read(d.join("o.plain.pk/dedans")).unwrap(), b"x");
        fs::write(d.join("f.plain"), b"fichier").unwrap();
        let t = Target::new(Some(&d.join("f.plain")), false, &pkg);
        assert_eq!(
            write_package(&pkg, &t, &never).unwrap_err().code,
            "output_error"
        );
        assert_eq!(fs::read(d.join("f.plain")).unwrap(), b"fichier");
        assert_no_part(&d);
    }

    /// Nom de plus de 65 535 octets : refus (`output_error`), rien de publié, aucun `.part`.
    #[test]
    fn nom_trop_long() {
        let long = format!("img/{}.lif", "a".repeat(0xFFFF));
        let d = scratch("long");
        let pkg = package(&[("version", b"1"), (long.as_str(), b"x")]);
        let t = Target::new(Some(&d.join("o.plain.pk")), false, &pkg);
        assert_eq!(
            write_package(&pkg, &t, &never).unwrap_err().code,
            "output_error"
        );
        assert_eq!(fs::read_dir(&d).unwrap().count(), 0);
    }

    /// Archive d'entrée en mémoire et paquet dont `sounds/s.mp3` y est référencé (#355).
    fn streamed_sample(mp3: &[u8]) -> (Archive<Cursor<Vec<u8>>>, Package<Data>) {
        let mut zip = Vec::new();
        let src = [("histoire/audios/s.mp3".to_owned(), mp3.to_vec())];
        write_zip(&mut zip, &src, ZipLimits::PYTHON, &never, &mut |_| Ok(())).unwrap();
        let archive = Archive::new(Cursor::new(zip), "in.zip".into()).unwrap();
        let entries = sample()
            .entries
            .into_iter()
            .map(|(n, d)| {
                let data = if n == "sounds/s.mp3" {
                    Data::Source {
                        name: "histoire/audios/s.mp3".to_owned(),
                        size: d.len() as u64,
                        crc: crc32fast::hash(&d),
                    }
                } else {
                    Data::Bytes(d)
                };
                (n, data)
            })
            .collect();
        let pkg = Package {
            entries,
            warnings: Vec::new(),
            title: "Titre".to_owned(),
            uuid: [0xAB; 16],
        };
        (archive, pkg)
    }

    /// MP3 relu en flux : mêmes octets que le paquet en mémoire, au `.plain.pk` comme au
    /// dossier ; panne d'écriture injectée au milieu de l'entrée relue (en-tête écrit, données
    /// non) puis avant son fichier dans le dossier : ancienne sortie intacte, aucun `.part`
    /// (l'échec de la relecture elle-même : `mp3_en_flux_modifie_refuse`).
    #[test]
    fn mp3_en_flux_et_panne_au_milieu() {
        let mem = sample();
        let (mut archive, pkg) = streamed_sample(&[0xFFu8; 3000]);
        let i = pkg
            .entries
            .iter()
            .position(|(n, _)| n == "sounds/s.mp3")
            .unwrap();
        for (k, step) in [Step::WritePk(i), Step::WriteDir(i)]
            .into_iter()
            .enumerate()
        {
            let d = scratch(&format!("flux-panne-{k}"));
            old_output(&d);
            let target = Target::new(Some(&d.join("o.plain.pk")), true, &pkg);
            let err = write_any(
                &pkg,
                &mut archive,
                &target,
                ZipLimits::PYTHON,
                &never,
                &mut (),
                &mut |s| {
                    if s == step {
                        Err(io::Error::other("panne injectee"))
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
            assert_eq!(err.code, "output_error", "{step:?}");
            assert_eq!(fs::read(d.join("o.plain.pk")).unwrap(), b"ancien paquet");
            assert_eq!(fs::read(d.join("o.plain/version")).unwrap(), b"ancien");
            assert_no_part(&d);
            // Reprise : sortie complète, identique au paquet en mémoire.
            write_package_from(&pkg, &mut archive, &target, &never, &mut ()).unwrap();
            assert_eq!(read_pk(&d.join("o.plain.pk")), mem.entries);
            for (name, data) in &mem.entries {
                assert_eq!(&fs::read(d.join("o.plain").join(name)).unwrap(), data);
            }
            assert_no_part(&d);
        }
        // Mêmes octets que l'écriture en mémoire.
        let (mut a, mut b) = (Vec::new(), Vec::new());
        let ok = |_| Ok(());
        zip_entries(
            &mut a,
            &pkg.entries,
            &mut archive,
            ZipLimits::PYTHON,
            &never,
            &mut { ok },
        )
        .unwrap();
        write_zip(&mut b, &mem.entries, ZipLimits::PYTHON, &never, &mut { ok }).unwrap();
        assert_eq!(a, b);
    }

    /// Archive d'entrée modifiée (ou entrée disparue) entre l'assemblage et l'écriture : refus
    /// (`archive_unreadable`), ancienne sortie intacte, aucun `.part` ; sans archive, refus.
    #[test]
    fn mp3_en_flux_modifie_refuse() {
        let (_, pkg) = streamed_sample(&[0xFFu8; 3000]);
        let mut changed = [0xFFu8; 3000];
        changed[1500] = 0;
        for (what, mp3) in [("contenu", &changed[..]), ("taille", &[0xFFu8; 2999][..])] {
            let (mut other, _) = streamed_sample(mp3);
            let d = scratch(&format!("flux-modifie-{what}"));
            old_output(&d);
            let target = Target::new(Some(&d.join("o.plain.pk")), true, &pkg);
            let e = write_package_from(&pkg, &mut other, &target, &never, &mut ()).unwrap_err();
            assert_eq!(e.code, "archive_unreadable", "{what}");
            assert_eq!(fs::read(d.join("o.plain.pk")).unwrap(), b"ancien paquet");
            assert_eq!(fs::read(d.join("o.plain/version")).unwrap(), b"ancien");
            assert_no_part(&d);
        }
        // Entrée absente de l'archive.
        let mut empty = Vec::new();
        let none: &[(String, Vec<u8>)] = &[];
        write_zip(&mut empty, none, ZipLimits::PYTHON, &never, &mut |_| Ok(())).unwrap();
        let mut empty = Archive::new(Cursor::new(empty), "vide.zip".into()).unwrap();
        let d = scratch("flux-absent");
        let target = Target::new(Some(&d.join("o.plain.pk")), false, &pkg);
        let e = write_package_from(&pkg, &mut empty, &target, &never, &mut ()).unwrap_err();
        assert_eq!(e.code, "archive_unreadable");
        assert_eq!(fs::read_dir(&d).unwrap().count(), 0);
        // Sans archive (chemin interne) : erreur interne, jamais d'entrée vide écrite.
        let e = write_any(
            &pkg,
            &mut NoSource,
            &target,
            ZipLimits::PYTHON,
            &never,
            &mut (),
            &mut |_| Ok(()),
        )
        .unwrap_err();
        assert_eq!(e.code, "internal_error");
        assert_eq!(fs::read_dir(&d).unwrap().count(), 0);
    }

    /// Un `.part` de dossier qui réapparaît (lien, jonction ou dossier planté) après le
    /// nettoyage est refusé, pas réutilisé.
    #[test]
    fn dossier_part_deja_present_refuse() {
        let pkg = sample();
        let d = scratch("plante");
        let tmp = part_path(&d.join("o.plain"));
        let t = Target::new(Some(&d.join("o.plain")), false, &pkg);
        let e = write_package_with(&pkg, &t, ZipLimits::PYTHON, &never, &mut (), &mut |s| {
            if s == Step::CreateDir {
                fs::create_dir(&tmp)?;
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(e.code, "output_error");
        assert!(!d.join("o.plain").exists());
        assert_no_part(&d);
    }

    #[test]
    fn drapeau_utf8_seulement_hors_ascii() {
        let pkg = package(&[("a.txt", b"1"), ("é.mp3", b"2")]);
        let mut out = Vec::new();
        write_zip(
            &mut out,
            &pkg.entries,
            ZipLimits::PYTHON,
            &never,
            &mut |_| Ok(()),
        )
        .unwrap();
        // Drapeaux de l'en-tête local, octets 6..8.
        assert_eq!(&out[6..8], &[0, 0]);
        let second = 30 + 5 + 1;
        assert_eq!(&out[second..second + 4], b"PK\x03\x04");
        assert_eq!(&out[second + 6..second + 8], &[0, 8]);
        // Heure et date fixes : 00:00:00, 1980-01-01.
        assert_eq!(&out[10..14], &[0, 0, 0x21, 0]);
    }
}
