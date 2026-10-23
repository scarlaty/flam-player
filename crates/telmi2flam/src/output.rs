//! Écriture de la sortie : `.plain.pk` (ZIP *stored*) et dossier `.plain`, la fin de `_convert`
//! (`tools/telmi2flam/telmi2flam.py`) avec `_commit`, `_clean_orphans` et `_discard`.
//!
//! Invariants (PORT-RUST.md §3) :
//! - **jamais de sortie partielle** : tout s'écrit dans un `.part` voisin de la cible, propre à
//!   l'écriture (`part_path`, privé au crate : `<cible>.<pid>.part`, puis `<cible>.<pid>-<n>.part`
//!   pour les écritures suivantes du même processus), puis le `.part` est publié par renommage
//!   (`std::fs::rename`, soit `MoveFileExW(REPLACE_EXISTING)` ou son équivalent POSIX sous
//!   Windows), relancé quelques fois si le fichier est verrouillé (antivirus). Le `.plain.pk`
//!   et chaque fichier du `.plain` sont vidés sur disque (`sync_all`) avant le renommage : une
//!   coupure de courant ne publie pas un fichier creux. Le dossier `.part` n'est jamais recréé
//!   en cours d'écriture, et son nombre de fichiers est vérifié avant publication : un `.part`
//!   effacé par un autre processus (qui l'a cru orphelin) fait échouer l'écriture ;
//! - **un échec ne détruit rien** : à chaque étape (création, écriture, `sync`, renommage,
//!   annulation), les `.part` sont supprimés et l'ancienne sortie reste intacte ; les types des
//!   cibles (fichier, dossier) sont vérifiés avant la première publication ;
//! - **aucune écriture hors de la destination** : garde zip-slip de Python sur les noms
//!   ([`check_names`]), et chaque dossier du `.plain` est créé composant par composant, un
//!   composant existant devant être un vrai dossier (ni lien ni jonction) ;
//! - **octets du ZIP identiques à `zipfile.writestr`** (Python 3.14 sous Windows) à horodatage
//!   égal : méthode *stored*, versions 20 (45 en ZIP64), système créateur 0 (MS-DOS),
//!   attributs externes `0o600 << 16`, bit 11 (UTF-8) seulement pour un nom non ASCII, ZIP64
//!   aux mêmes seuils que `zipfile`. Seul écart : l'horodatage est fixé au 1980-01-01 00:00
//!   (Python prend l'heure locale, PORT-RUST.md §4.1) ; le lecteur du player
//!   (`src/formats/pk_reader.c`) l'ignore ;
//! - **MP3 référencés** ([`Data::Source`], #355) : relus dans l'archive d'entrée un à un, au
//!   moment de leur écriture (`.plain.pk` puis dossier `.plain`), par tours de [`COPY_CHUNK`]
//!   octets (annulation consultée entre deux tours), et refusés (`archive_unreadable`, sortie
//!   non publiée) si leur taille ou leur CRC-32 diffère de ceux de l'assemblage. L'archive
//!   d'entrée est fermée avant la première publication (#402).
//!
//! Limites reprises de Python : le dossier `.plain` existant est mis à jour fichier par fichier
//! (`_commit`), donc sans atomicité, et les publications du `.plain.pk` puis du `.plain` ne sont
//! pas atomiques ensemble. Le launcher n'utilise que le `.plain.pk`. Le dossier parent n'est
//! vidé sur disque après le renommage que sous POSIX (NTFS journalise le renommage).
//!
//! Écarts voulus avec Python : un `.part` laissé par un processus mort qui portait **notre**
//! PID est supprimé avant l'écriture (Python le réutiliserait et publierait ses fichiers
//! périmés dans le `.plain`) ; deux écritures du même processus vers la même cible ont chacune
//! leur `.part` ; un `.plain` à publier sur un fichier existant (et non un dossier) est refusé
//! sans rien remplacer, `.plain.pk` compris ; un PID orphelin n'est reconnu qu'en chiffres
//! ASCII (Python accepte tout `str.isdigit()`) ; un lien symbolique orphelin est supprimé sans
//! être suivi.

use std::cell::RefCell;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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

/// Taille des tours de relecture d'une entrée [`Data::Source`] (#402) : la mémoire reste
/// bornée par ce tour, et l'annulation est consultée entre deux tours.
pub(crate) const COPY_CHUNK: usize = 256 * 1024;

/// Destinataire des octets d'une entrée, tour par tour.
pub(crate) type Sink<'a> = &'a mut dyn FnMut(&[u8]) -> Result<(), ZipStop>;

/// Archive d'où relire les entrées [`Data::Source`].
pub(crate) trait Source {
    /// Copie l'entrée `name` vers `sink` par tours d'au plus [`COPY_CHUNK`] octets ;
    /// `cancelled` est consulté avant chaque tour sauf le premier. Refus
    /// (`archive_unreadable`) si la taille ou le CRC-32 diffère de `size` et `crc` : le refus
    /// peut survenir **après** que `sink` a reçu des octets (l'appelant jette alors sa sortie).
    fn copy(
        &mut self,
        name: &str,
        size: u64,
        crc: u32,
        cancelled: &dyn Fn() -> bool,
        sink: Sink<'_>,
    ) -> Result<(), ZipStop>;
}

/// Une référence suffit (tests, ou archive gardée par l'appelant).
impl<S: Source + ?Sized> Source for &mut S {
    fn copy(
        &mut self,
        name: &str,
        size: u64,
        crc: u32,
        cancelled: &dyn Fn() -> bool,
        sink: Sink<'_>,
    ) -> Result<(), ZipStop> {
        (**self).copy(name, size, crc, cancelled, sink)
    }
}

/// Pas d'archive : toute entrée référencée est refusée.
pub(crate) struct NoSource;

impl Source for NoSource {
    fn copy(
        &mut self,
        name: &str,
        _: u64,
        _: u32,
        _: &dyn Fn() -> bool,
        _: Sink<'_>,
    ) -> Result<(), ZipStop> {
        Err(ZipStop::Source(ConvError::internal_error(format!(
            "{name} : entree referencee sans archive"
        ))))
    }
}

impl<R: Read + Seek> Source for Archive<R> {
    fn copy(
        &mut self,
        name: &str,
        size: u64,
        crc: u32,
        cancelled: &dyn Fn() -> bool,
        sink: Sink<'_>,
    ) -> Result<(), ZipStop> {
        let changed = || {
            ZipStop::Source(ConvError::archive_unreadable(format!(
                "{name} : entree modifiee ou disparue depuis sa lecture"
            )))
        };
        let mut seen = 0u64;
        let mut hasher = crc32fast::Hasher::new();
        let found = self.read_chunks(name, COPY_CHUNK, &mut |chunk: &[u8]| {
            if seen > 0 && cancelled() {
                return Err(ZipStop::Cancelled);
            }
            seen += chunk.len() as u64;
            // Entrée devenue plus grande : arrêt avant d'écrire au-delà de la taille annoncée.
            if seen > size {
                return Err(changed());
            }
            hasher.update(chunk);
            sink(chunk)
        })?;
        if !found || seen != size || hasher.finalize() != crc {
            return Err(changed());
        }
        Ok(())
    }
}

/// Contenu d'une entrée à écrire : en mémoire, ou relu dans l'archive d'entrée.
pub(crate) trait Body {
    /// Taille annoncée dans l'en-tête.
    fn size(&self) -> u64;
    /// CRC-32 annoncé dans l'en-tête : celui des octets, ou celui de l'assemblage pour une
    /// entrée référencée (vérifié pendant la copie).
    fn crc(&self) -> u32;
    /// Envoie le contenu à `sink` (en un tour pour des octets en mémoire).
    fn copy(
        &self,
        src: &mut dyn Source,
        cancelled: &dyn Fn() -> bool,
        sink: Sink<'_>,
    ) -> Result<(), ZipStop>;
}

impl Body for Vec<u8> {
    fn size(&self) -> u64 {
        self.len() as u64
    }
    fn crc(&self) -> u32 {
        crc32fast::hash(self)
    }
    fn copy(
        &self,
        _: &mut dyn Source,
        _: &dyn Fn() -> bool,
        sink: Sink<'_>,
    ) -> Result<(), ZipStop> {
        sink(self)
    }
}

impl Body for Data {
    fn size(&self) -> u64 {
        Data::size(self)
    }
    fn crc(&self) -> u32 {
        match self {
            Data::Bytes(b) => crc32fast::hash(b),
            Data::Source { crc, .. } => *crc,
        }
    }
    fn copy(
        &self,
        src: &mut dyn Source,
        cancelled: &dyn Fn() -> bool,
        sink: Sink<'_>,
    ) -> Result<(), ZipStop> {
        match self {
            Data::Bytes(b) => sink(b),
            Data::Source { name, size, crc } => src.copy(name, *size, *crc, cancelled, sink),
        }
    }
}

impl From<io::Error> for ZipStop {
    fn from(e: io::Error) -> Self {
        ZipStop::Io(e)
    }
}

impl From<ConvError> for ZipStop {
    fn from(e: ConvError) -> Self {
        ZipStop::Source(e)
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

/// [`write_zip`] pour des entrées en mémoire ou relues dans `src` (une à la fois, par tours,
/// après l'en-tête local, qui porte la taille et le CRC de l'assemblage).
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
        let size = body.size();
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
        let crc = body.crc();
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
        let start = w.pos;
        body.copy(src, cancelled, &mut |chunk: &[u8]| Ok(w.put(chunk)?))?;
        if w.pos - start != size {
            return Err(ZipStop::Source(ConvError::internal_error(format!(
                "{} : taille ecrite differente de l'en-tete",
                String::from_utf8_lossy(name)
            ))));
        }
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

/// Numéro d'écriture dans le processus : deux écritures (fils, appels successifs) vers la même
/// cible n'ont jamais le même `.part` (#357).
static WRITES: AtomicU64 = AtomicU64::new(0);

/// Temporaire voisin de l'écriture n° `call` du processus : `<cible>.<pid>.part` pour la
/// première (le nom de Python, que son `_clean_orphans` sait nettoyer), `<cible>.<pid>-<n>.part`
/// pour les suivantes. Le `-` (jamais un `.`) rend le nom non ambigu : le suffixe est toujours
/// le dernier composant pointé avant `.part`.
pub(crate) fn part_path(target: &Path, call: u64) -> PathBuf {
    let pid = std::process::id();
    if call == 0 {
        with_suffix(target, &format!(".{pid}.part"))
    } else {
        with_suffix(target, &format!(".{pid}-{call}.part"))
    }
}

/// Étapes où un test injecte une panne.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// Création du `.part` du `.plain.pk`.
    CreatePk,
    /// Au milieu de l'entrée `i` du `.plain.pk` (en-tête écrit, données non).
    WritePk(usize),
    /// Écriture réelle dans le fichier `.part` du `.plain.pk`, à la position donnée (vidages
    /// du tampon, dont le dernier, `BufWriter::into_inner`).
    FilePk(u64),
    /// `sync_all` du `.plain.pk`.
    SyncPk,
    /// Création du dossier `.part` du `.plain`.
    CreateDir,
    /// Fichier `i` du dossier `.plain`.
    WriteDir(usize),
    /// `sync_all` du fichier `i` du dossier `.plain`.
    SyncDir(usize),
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
/// entrées [`Data::Source`] sont relues dans `archive` (celle de l'assemblage), une à la fois
/// et par tours. L'archive est fermée (rendue) avant la publication (#402 : sous Windows, un
/// fichier encore ouvert peut bloquer le renommage, entrée et sortie sur le même volume).
pub fn write_package_from<R: Read + Seek>(
    package: &Package<Data>,
    archive: Archive<R>,
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
    write_any(package, NoSource, target, limits, cancelled, progress, hook)
}

/// Écriture complète ; `src` est rendu (fermé) avant la première publication.
pub(crate) fn write_any<D: Body, S: Source>(
    package: &Package<D>,
    src: S,
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
    let call = WRITES.fetch_add(1, Ordering::Relaxed);
    // Garde RAII : les `.part` sont supprimés et désinscrits du registre à la sortie de cette
    // fonction, y compris par une panique qui se déroule (bibliothèque, `panic = "unwind"`).
    let mut guard = PartGuard(Vec::new());
    let mut src = Some(src);
    let mut verified = false;
    let mut result = write_and_commit(
        package,
        &mut src,
        target,
        call,
        limits,
        cancelled,
        progress,
        hook,
        &mut guard.0,
        &mut verified,
    );
    // Cause première : une panne de sortie qui survient avant que l'archive d'entrée ait été
    // relue en entier masquerait une archive illisible (`archive_unreadable`). Relecture
    // seulement en cas d'échec, l'archive étant encore ouverte.
    // Inutile si toutes les entrées ont déjà été relues et vérifiées (`verified`).
    if let (Err(Fail::Io(io)), Some(src), false) = (&result, src.as_mut(), verified) {
        if let Some(e) = first_source_error(package, src, cancelled) {
            let mut e = e;
            e.message = format!("{} (ecriture de la sortie : {io})", e.message);
            result = Err(Fail::Conv(e));
        }
    }
    drop(src);
    drop(guard);
    match result {
        Ok(()) => Ok(target.published()),
        Err(Fail::Conv(e)) => Err(e),
        Err(Fail::Io(e)) => Err(ConvError::output_error(format!(
            "ecriture de {} impossible : {e}",
            target.out_path.display()
        ))),
    }
}

/// Temporaires d'une écriture : supprimés puis retirés du registre au `Drop`.
struct PartGuard(Vec<PathBuf>);

impl Drop for PartGuard {
    fn drop(&mut self) {
        discard(&self.0);
        untrack(&self.0);
    }
}

/// Première erreur de source rencontrée en relisant les entrées dans l'archive d'entrée (le plus
/// souvent `archive_unreadable`, mais toute erreur de la source est rendue), sans rien écrire.
/// L'annulation et les entrées en mémoire ne comptent pas.
fn first_source_error<D: Body>(
    package: &Package<D>,
    src: &mut dyn Source,
    cancelled: &dyn Fn() -> bool,
) -> Option<ConvError> {
    for (_, body) in &package.entries {
        if cancelled() {
            return None;
        }
        if let Err(ZipStop::Source(e)) = body.copy(src, cancelled, &mut |_| Ok(())) {
            return Some(e);
        }
    }
    None
}

#[derive(Debug)]
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

/// Appelle le crochet d'injection de panne partagé.
fn fire(hook: &RefCell<Hook<'_>>, step: Step) -> io::Result<()> {
    let mut guard = hook.borrow_mut();
    let f: &mut dyn FnMut(Step) -> io::Result<()> = &mut **guard;
    f(step)
}

/// Fichier `.part` du `.plain.pk` : chaque écriture réelle passe d'abord par le crochet
/// ([`Step::FilePk`]), pour simuler un disque qui lâche après `k` octets.
struct Tap<'a, 'h> {
    file: fs::File,
    hook: &'a RefCell<Hook<'h>>,
    pos: u64,
}

impl Write for Tap<'_, '_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        fire(self.hook, Step::FilePk(self.pos))?;
        let n = self.file.write(buf)?;
        self.pos += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[allow(clippy::too_many_arguments)]
fn write_and_commit<D: Body, S: Source>(
    package: &Package<D>,
    src_slot: &mut Option<S>,
    target: &Target,
    call: u64,
    limits: ZipLimits,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn Progress,
    hook: Hook<'_>,
    tmps: &mut Vec<PathBuf>,
    verified: &mut bool,
) -> Result<(), Fail> {
    let hook = RefCell::new(hook);
    let mut commits = Vec::new();
    let Some(src) = src_slot.as_mut() else {
        return Err(Fail::Conv(ConvError::internal_error("archive deja fermee")));
    };
    if !target.dir_only {
        let abs = std::path::absolute(&target.out_path)?;
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = part_path(&target.out_path, call);
        tmps.push(tmp.clone());
        claim(&tmp);
        fire(&hook, Step::CreatePk)?;
        let file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        let tap = Tap {
            file,
            hook: &hook,
            pos: 0,
        };
        let mut buf = BufWriter::with_capacity(1 << 20, tap);
        // Une étape par entrée écrite (après `writestr` en Python) : l'entrée `i - 1` est
        // complète quand l'en-tête de l'entrée `i` est écrit, la dernière au retour.
        let written = zip_entries(
            &mut buf,
            &package.entries,
            &mut *src,
            limits,
            cancelled,
            &mut |i| {
                if i > 0 {
                    progress.tick(Phase::Zip);
                }
                fire(&hook, Step::WritePk(i))
            },
        );
        if let Err(e) = written {
            // Tampon jeté sans être écrit : le `.part` sera supprimé (pas de vidage inutile ni de
            // nouvel appel au crochet).
            let _ = buf.into_parts();
            return Err(e.into());
        }
        if !package.entries.is_empty() {
            progress.tick(Phase::Zip);
        }
        let file = buf.into_inner().map_err(|e| e.into_error())?.file;
        // Toutes les entrées relues et vérifiées si aucun `.plain` ne les relit.
        *verified = target.plain_dir.is_none();
        fire(&hook, Step::SyncPk)?;
        file.sync_all()?;
        drop(file);
        commits.push((tmp, target.out_path.clone()));
    }
    if let Some(plain_dir) = &target.plain_dir {
        let tmp = part_path(plain_dir, call);
        tmps.push(tmp.clone());
        claim(&tmp);
        fire(&hook, Step::CreateDir)?;
        let abs = std::path::absolute(&tmp)?;
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }
        // `create_dir` et non `create_dir_all` : un lien ou une jonction placé à ce nom entre
        // `discard` et ici est refusé au lieu d'être suivi (écriture hors de la destination).
        fs::create_dir(&abs)?;
        for (i, (name, body)) in package.entries.iter().enumerate() {
            if cancelled() {
                return Err(Fail::Conv(ConvError::cancelled()));
            }
            let mut parts: Vec<&str> = name.split('/').collect();
            let file_name = parts.pop().unwrap_or_default();
            let parent = make_dirs(&abs, &parts)?;
            fire(&hook, Step::WriteDir(i))?;
            // `create_new` : jamais d'écriture à travers un lien déjà présent.
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(parent.join(file_name))?;
            body.copy(&mut *src, cancelled, &mut |chunk: &[u8]| {
                Ok(f.write_all(chunk)?)
            })?;
            fire(&hook, Step::SyncDir(i))?;
            f.sync_all()?;
            drop(f);
            progress.tick(Phase::Zip);
        }
        *verified = true;
        // Un `.part` effacé (même en partie) par un autre processus qui l'a cru orphelin
        // (partage réseau, autre machine) n'est jamais publié incomplet.
        let found = count_files(&abs)?;
        if found != package.entries.len() as u64 {
            return Err(Fail::Io(io::Error::other(format!(
                "{} : {found} fichier(s) au lieu de {}, dossier modifie pendant l'ecriture",
                abs.display(),
                package.entries.len()
            ))));
        }
        commits.push((tmp, plain_dir.clone()));
    }
    // Archive d'entrée fermée avant toute publication (#402).
    drop(src_slot.take());
    if cancelled() {
        return Err(Fail::Conv(ConvError::cancelled()));
    }
    // Types des cibles vérifiés avant la première publication : un `.plain` qui est un fichier
    // ne laisse pas un `.plain.pk` publié seul.
    for (tmp, fin) in &commits {
        check_target(tmp, fin)?;
    }
    for (tmp, fin) in &commits {
        commit(tmp, fin, &mut |s| fire(&hook, s))?;
        sync_parent(fin);
    }
    Ok(())
}

/// Crée les dossiers `parts` sous `root`, un composant à la fois, et rend le dernier. `root`
/// n'est jamais recréé : s'il a disparu, échec (`NotFound`). Un composant déjà présent doit
/// être un vrai dossier : un lien ou une jonction est refusé (`invalid_content`) sans recourir
/// à `fs::canonicalize` (indisponible sur certains volumes virtuels). Composant `.` ignoré.
fn make_dirs(root: &Path, parts: &[&str]) -> Result<PathBuf, Fail> {
    let mut cur = root.to_path_buf();
    for part in parts {
        if *part == "." {
            continue;
        }
        cur.push(part);
        match fs::create_dir(&cur) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                let meta = fs::symlink_metadata(&cur)?;
                if meta.file_type().is_symlink() {
                    return Err(Fail::Conv(ConvError::invalid_content(format!(
                        "ecriture hors du dossier .plain refusee : {}",
                        cur.display()
                    ))));
                }
                if !meta.is_dir() {
                    return Err(Fail::Io(e));
                }
            }
            Err(e) => return Err(Fail::Io(e)),
        }
    }
    Ok(cur)
}

/// Nombre de fichiers sous `dir` (liens comptés sans être suivis).
fn count_files(dir: &Path) -> io::Result<u64> {
    let mut n = 0;
    for e in fs::read_dir(dir)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            n += count_files(&e.path())?;
        } else {
            n += 1;
        }
    }
    Ok(n)
}

/// Vide sur disque l'entrée du dossier parent après un renommage (POSIX). Sous Windows, NTFS
/// journalise le renommage et un dossier ne s'ouvre pas en écriture : rien à faire. Au mieux :
/// la sortie est déjà publiée, une erreur ici n'est pas remontée.
fn sync_parent(path: &Path) {
    #[cfg(unix)]
    if let Some(parent) = std::path::absolute(path)
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Refus d'un temporaire dont la cible est d'un autre type (dossier contre fichier).
fn check_target(tmp: &Path, fin: &Path) -> io::Result<()> {
    let tmp_is_dir = fs::symlink_metadata(tmp)?.is_dir();
    // Lien cassé : jugé sur le lien lui-même (un dossier ne le remplace pas).
    match fs::metadata(fin).or_else(|_| fs::symlink_metadata(fin)) {
        // Python échoue aussi (`os.replace` d'un dossier sur un fichier, ou l'inverse) ; le
        // refus explicite évite qu'un renommage POSIX remplace quand même la cible.
        Ok(m) if tmp_is_dir && !m.is_dir() => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} existe et n'est pas un dossier", fin.display()),
        )),
        Ok(m) if !tmp_is_dir && m.is_dir() => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} est un dossier", fin.display()),
        )),
        _ => Ok(()),
    }
}

/// `_commit` : publie un temporaire. Dossier vers dossier existant : fichiers remplacés un à
/// un sans supprimer le reste (comportement historique), puis temporaire supprimé.
fn commit(tmp: &Path, fin: &Path, hook: Hook<'_>) -> io::Result<()> {
    check_target(tmp, fin)?;
    let tmp_is_dir = fs::symlink_metadata(tmp)?.is_dir();
    if tmp_is_dir && fs::metadata(fin).is_ok_and(|m| m.is_dir()) {
        merge_dir(tmp, fin, hook)?;
        let _ = fs::remove_dir_all(tmp);
        return Ok(());
    }
    rename_retry(tmp, fin, hook)
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

/// Supprime un éventuel reste à ce nom puis inscrit `tmp`, sous le verrou du registre :
/// [`clean_orphans`] ne peut pas supprimer entre les deux un `.part` qui vient d'être créé.
fn claim(tmp: &Path) {
    let mut reg = pending();
    discard(std::slice::from_ref(&tmp.to_path_buf()));
    reg.push(tmp.to_path_buf());
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
///
/// Limite : sur FAT/exFAT (clé USB, carte de la Flam), une panique pendant l'écriture du
/// dossier (`WriteDir`) peut laisser `.plain.<pid>.part` : le fichier en cours d'écriture est
/// encore ouvert, et sans la suppression POSIX de NTFS, Windows ne l'efface pas avant sa
/// fermeture (la sortie du processus), donc le dossier n'est pas vide pour `remove_dir_all`.
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

/// `_clean_orphans` : supprime les `<cible>.<pid>.part` et `<cible>.<pid>-<n>.part` voisins
/// dont le PID n'est plus vivant (une conversion concurrente vivante est épargnée). Arrêt
/// brutal puis reprise : le `.part` laissé disparaît à la conversion suivante vers la même
/// cible. Notre propre PID (porté avant nous par un processus mort) : supprimé, sauf s'il est
/// inscrit au registre des temporaires (écriture en cours dans ce processus). Un PID réutilisé
/// par un autre processus vivant empêche ce nettoyage (limite connue). Un `.part` d'une autre
/// machine sur un partage réseau peut être jugé mort à tort : l'écriture qui le possède échoue
/// alors sans rien publier ([`write_package`] ne recrée jamais un `.part` disparu).
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
        let Some(middle) = bytes
            .strip_prefix(prefix.as_slice())
            .and_then(|r| r.strip_suffix(b".part"))
        else {
            continue;
        };
        // `<pid>` (Python, première écriture du processus) ou `<pid>-<n>` (écritures suivantes).
        let (pid, call) = match middle.iter().position(|&b| b == b'-') {
            Some(i) => (&middle[..i], Some(&middle[i + 1..])),
            None => (middle, None),
        };
        let digits = |s: &[u8]| !s.is_empty() && s.iter().all(u8::is_ascii_digit);
        if !digits(pid) || call.is_some_and(|c| !digits(c)) {
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
        let path = dir.join(&name);
        if pid == std::process::id() {
            // Verrou tenu de la consultation à la suppression : une écriture de ce processus
            // inscrit son `.part` sous ce même verrou ([`claim`]). Comparaison par nom (il porte
            // le PID et le numéro d'écriture), quelle que soit l'écriture du chemin (8.3, `\\?\`).
            let reg = pending();
            if !reg.iter().any(|p| p.file_name() == Some(name.as_os_str())) {
                discard(&[path]);
            }
        } else if !pid_alive(pid) {
            discard(&[path]);
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
            stats: Default::default(),
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
            part_path(Path::new("a/b.plain.pk"), 0),
            PathBuf::from(format!("a/b.plain.pk.{}.part", std::process::id()))
        );
        assert_eq!(
            part_path(Path::new("a/b.plain.pk"), 12),
            PathBuf::from(format!("a/b.plain.pk.{}-12.part", std::process::id()))
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

    /// `.part` de `base` inscrit au registre (écriture en cours de ce processus), s'il y en a.
    fn tracked_part(base: &Path) -> Option<PathBuf> {
        let prefix = format!("{}.{}", base.display(), std::process::id());
        pending()
            .iter()
            .find(|p| {
                let s = p.display().to_string();
                let Some(rest) = s.strip_prefix(&prefix) else {
                    return false;
                };
                let Some(call) = rest.strip_suffix(".part") else {
                    return false;
                };
                call.is_empty()
                    || call
                        .strip_prefix('-')
                        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            })
            .cloned()
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
        let pk = target.out_path.clone();
        let dir = target.plain_dir.clone().unwrap();
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
                    assert!(tracked_part(&pk).is_some() && tracked_part(&dir).is_some());
                    seen.set(true);
                }
                Ok(())
            },
        )
        .unwrap();
        assert!(seen.get());
        assert!(
            tracked_part(&pk).is_none() && tracked_part(&dir).is_none(),
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
        assert!(
            tracked_part(&pk).is_none() && tracked_part(&dir).is_none(),
            "échec : retirés"
        );
        // Panique qui se déroule au milieu du `.plain.pk` : la garde RAII supprime le `.part`
        // et le désinscrit (#416). Avec `panic = "abort"` (binaire), c'est `discard_pending`
        // (jamais appelé ici : il viderait les écritures des autres tests).
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
        assert!(tracked_part(&pk).is_none(), "désinscrit au Drop");
        assert_no_part(&d);
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

    /// PID d'un processus terminé et non réattribué : Windows peut redonner aussitôt le numéro
    /// d'un fils attendu à un autre processus, on réessaie alors avec un autre fils (#357).
    fn dead_pid() -> u32 {
        for _ in 0..20 {
            let child = long_child();
            let pid = child.0.id();
            drop(child);
            if !pid_alive(pid) {
                return pid;
            }
        }
        panic!("aucun PID mort stable en 20 essais");
    }

    /// Arrêt brutal puis reprise : les `.part` d'un PID mort (fichier tronqué, dossier, nom de
    /// Python `<pid>` ou suivant `<pid>-<n>`) sont supprimés par la conversion suivante ;
    /// celui d'une conversion vivante est épargné, de même que les noms qui ne sont pas
    /// `<cible>.<chiffres>[-<chiffres>].part`.
    #[test]
    fn orphelins_apres_arret_brutal() {
        let pkg = sample();
        let d = scratch("orphelins");
        let dead_pid = dead_pid();
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
        fs::write(d.join(format!("o.plain.pk.{dead_pid}-3.part")), b"x").unwrap();
        fs::create_dir_all(d.join(format!("o.plain.{dead_pid}.part/img"))).unwrap();
        fs::write(d.join(format!("o.plain.{dead_pid}.part/img/a.lif")), b"x").unwrap();
        // Notre propre PID (processus mort qui portait le même numéro), non inscrit au
        // registre : supprimé.
        let me = std::process::id();
        fs::write(d.join(format!("o.plain.pk.{me}.part")), b"vieux").unwrap();
        fs::write(d.join(format!("o.plain.pk.{me}-99999.part")), b"vieux").unwrap();
        let kept = [
            format!("o.plain.pk.{alive_pid}.part"),
            format!("o.plain.pk.{alive_pid}-2.part"),
            "o.plain.pk.abc.part".to_owned(),
            "o.plain.pk..part".to_owned(),
            format!("o.plain.pk.{dead_pid}-.part"),
            format!("o.plain.pk.{dead_pid}-x.part"),
            format!("o.plain.pk.-{dead_pid}.part"),
            format!("o.plain.pk.{dead_pid}.1.part"),
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
                        size: mp3.len() as u64,
                        crc: crc32fast::hash(mp3),
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
            stats: Default::default(),
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
            write_any(
                &pkg,
                &mut archive,
                &target,
                ZipLimits::PYTHON,
                &never,
                &mut (),
                &mut |_| Ok(()),
            )
            .unwrap();
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
            let (other, _) = streamed_sample(mp3);
            let d = scratch(&format!("flux-modifie-{what}"));
            old_output(&d);
            let target = Target::new(Some(&d.join("o.plain.pk")), true, &pkg);
            let e = write_package_from(&pkg, other, &target, &never, &mut ()).unwrap_err();
            assert_eq!(e.code, "archive_unreadable", "{what}");
            assert_eq!(fs::read(d.join("o.plain.pk")).unwrap(), b"ancien paquet");
            assert_eq!(fs::read(d.join("o.plain/version")).unwrap(), b"ancien");
            assert_no_part(&d);
        }
        // Entrée absente de l'archive.
        let mut empty = Vec::new();
        let none: &[(String, Vec<u8>)] = &[];
        write_zip(&mut empty, none, ZipLimits::PYTHON, &never, &mut |_| Ok(())).unwrap();
        let empty = Archive::new(Cursor::new(empty), "vide.zip".into()).unwrap();
        let d = scratch("flux-absent");
        let target = Target::new(Some(&d.join("o.plain.pk")), false, &pkg);
        let e = write_package_from(&pkg, empty, &target, &never, &mut ()).unwrap_err();
        assert_eq!(e.code, "archive_unreadable");
        assert_eq!(fs::read_dir(&d).unwrap().count(), 0);
        // Sans archive (chemin interne) : erreur interne, jamais d'entrée vide écrite.
        let e = write_any(
            &pkg,
            NoSource,
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
        let t = Target::new(Some(&d.join("o.plain")), false, &pkg);
        let e = write_package_with(&pkg, &t, ZipLimits::PYTHON, &never, &mut (), &mut |s| {
            if s == Step::CreateDir {
                fs::create_dir(tracked_part(&d.join("o.plain")).unwrap())?;
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

    /// Deux écritures du même processus vers la même cible (#357) : la seconde, lancée au
    /// milieu de la première, a son propre `.part` ; chacune publie un paquet complet, la
    /// dernière publiée gagne, aucun `.part` ne reste.
    #[test]
    fn ecritures_imbriquees_meme_cible() {
        let d = scratch("imbriquees");
        let a = package(&[("version", b"A"), ("img/a.lif", &[1u8; 5000])]);
        let b = package(&[("version", b"B"), ("img/b.lif", &[2u8; 7000])]);
        for plain in [false, true] {
            let ta = Target::new(Some(&d.join("o.plain.pk")), plain, &a);
            let tb = Target::new(Some(&d.join("o.plain.pk")), plain, &b);
            let nested = Cell::new(false);
            write_package_with(&a, &ta, ZipLimits::PYTHON, &never, &mut (), &mut |s| {
                if s == Step::WritePk(1) && !nested.replace(true) {
                    write_package(&b, &tb, &never).unwrap();
                    assert_eq!(read_pk(&d.join("o.plain.pk")), b.entries);
                }
                Ok(())
            })
            .unwrap();
            assert!(nested.get());
            assert_eq!(read_pk(&d.join("o.plain.pk")), a.entries, "plain {plain}");
            if plain {
                assert_eq!(fs::read(d.join("o.plain/version")).unwrap(), b"A");
                assert_eq!(fs::read(d.join("o.plain/img/b.lif")).unwrap(), [2u8; 7000]);
            }
            assert_no_part(&d);
        }
    }

    /// Paquet dont le `.plain.pk` dépasse le tampon d'écriture (1 Mio) : plusieurs écritures
    /// réelles dans le fichier.
    fn big_package() -> Package {
        package(&[
            ("version", b"1"),
            ("sounds/a.mp3", &[1u8; 1_500_000]),
            ("sounds/b.mp3", &[2u8; 1_500_000]),
            ("img/c.lif", &[3u8; 700_000]),
        ])
    }

    /// Disque qui lâche au milieu du fichier (#357) : panne à chaque écriture réelle du
    /// `.part` (dont le vidage final du tampon), erreur quelconque ou disque plein : ancienne
    /// sortie intacte, aucun `.part`.
    #[test]
    fn panne_du_fichier_apres_k_octets() {
        let pkg = big_package();
        let d = scratch("fichier-k");
        old_output(&d);
        let target = Target::new(Some(&d.join("o.plain.pk")), false, &pkg);
        let offsets = RefCell::new(Vec::new());
        write_package_with(
            &pkg,
            &target,
            ZipLimits::PYTHON,
            &never,
            &mut (),
            &mut |s| {
                if let Step::FilePk(k) = s {
                    offsets.borrow_mut().push(k);
                }
                Ok(())
            },
        )
        .unwrap();
        let offsets = offsets.into_inner();
        assert!(offsets.len() >= 3, "{offsets:?}");
        assert_eq!(offsets[0], 0);
        assert!(offsets.windows(2).all(|w| w[0] < w[1]));
        for &k in &offsets {
            for disk_full in [false, true] {
                old_output(&d);
                let e = write_package_with(
                    &pkg,
                    &target,
                    ZipLimits::PYTHON,
                    &never,
                    &mut (),
                    &mut |s| match s {
                        Step::FilePk(o) if o == k && disk_full => {
                            Err(io::Error::from_raw_os_error(112))
                        }
                        Step::FilePk(o) if o == k => Err(io::Error::other("panne")),
                        _ => Ok(()),
                    },
                )
                .unwrap_err();
                assert_eq!(e.code, "output_error", "k = {k}");
                assert_eq!(fs::read(d.join("o.plain.pk")).unwrap(), b"ancien paquet");
                assert_no_part(&d);
            }
        }
    }

    /// Écrivain qui accepte `left` octets puis échoue (disque plein).
    struct FailAfter {
        out: Vec<u8>,
        left: usize,
    }

    impl Write for FailAfter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.left == 0 {
                return Err(io::Error::from_raw_os_error(112));
            }
            let n = buf.len().min(self.left);
            self.out.extend_from_slice(&buf[..n]);
            self.left -= n;
            Ok(n)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// `write_zip` (et la copie en flux) sur un écrivain qui lâche après `k` octets, pour
    /// `k` de 0 à la taille complète : erreur d'entrée-sortie, jamais de succès tronqué.
    #[test]
    fn write_zip_ecrivain_qui_lache() {
        let pkg = sample();
        let mut full = Vec::new();
        write_zip(
            &mut full,
            &pkg.entries,
            ZipLimits::PYTHON,
            &never,
            &mut |_| Ok(()),
        )
        .unwrap();
        let (mut archive, streamed) = streamed_sample(&[0xFFu8; 3000]);
        for k in 0..=full.len() {
            let mut w = FailAfter {
                out: Vec::new(),
                left: k,
            };
            let r = write_zip(&mut w, &pkg.entries, ZipLimits::PYTHON, &never, &mut |_| {
                Ok(())
            });
            let mut s = FailAfter {
                out: Vec::new(),
                left: k,
            };
            let rs = zip_entries(
                &mut s,
                &streamed.entries,
                &mut archive,
                ZipLimits::PYTHON,
                &never,
                &mut |_| Ok(()),
            );
            if k == full.len() {
                r.unwrap();
                rs.unwrap();
                assert!(w.out == full && s.out == full);
            } else {
                assert!(matches!(r, Err(ZipStop::Io(_))), "k = {k}");
                assert!(matches!(rs, Err(ZipStop::Io(_))), "flux, k = {k}");
                assert_eq!(w.out.len(), k);
            }
        }
    }

    /// Dossiers créés composant par composant (#357) : la racine (le `.part`) n'est jamais
    /// recréée, `.` est ignoré, un fichier ou un lien à la place d'un dossier est refusé.
    #[test]
    fn dossiers_sans_recreer_la_racine() {
        let d = scratch("make-dirs");
        let root = d.join("racine");
        assert!(matches!(make_dirs(&root, &["a", "b"]), Err(Fail::Io(_))));
        assert!(!root.exists(), "racine recréée");
        fs::create_dir(&root).unwrap();
        let p = make_dirs(&root, &["a", ".", "b"]).unwrap();
        assert_eq!(p, root.join("a").join("b"));
        assert!(p.is_dir());
        assert_eq!(make_dirs(&root, &["a", "b"]).unwrap(), p);
        assert_eq!(make_dirs(&root, &[]).unwrap(), root);
        fs::write(root.join("f"), b"x").unwrap();
        assert!(matches!(make_dirs(&root, &["f", "g"]), Err(Fail::Io(_))));
        // Lien symbolique ou jonction vers l'extérieur : refusé, rien d'écrit dehors.
        let outside = d.join("dehors");
        fs::create_dir(&outside).unwrap();
        let link = root.join("lien");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&outside, &link).is_ok();
        #[cfg(windows)]
        let made = std::process::Command::new("cmd")
            .arg("/c")
            .arg("mklink")
            .arg("/J")
            .arg(&link)
            .arg(&outside)
            .stdout(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        #[cfg(not(any(unix, windows)))]
        let made = false;
        if made {
            match make_dirs(&root, &["lien", "x"]) {
                Err(Fail::Conv(e)) => assert_eq!(e.code, "invalid_content"),
                _ => panic!("lien suivi"),
            }
            assert!(!outside.join("x").exists());
            discard(&[link]);
        }
        assert!(outside.exists());
    }

    /// Composant `.` dans un nom : écrit au même endroit que Python (`os.path.join`), le nom
    /// reste tel quel dans le `.plain.pk`.
    #[test]
    fn point_dans_un_nom() {
        let d = scratch("point");
        let pkg = package(&[("version", b"1"), ("a/./b.txt", b"2")]);
        let t = Target::new(Some(&d.join("o.plain.pk")), true, &pkg);
        write_package(&pkg, &t, &never).unwrap();
        assert_eq!(read_pk(&d.join("o.plain.pk")), pkg.entries);
        assert_eq!(fs::read(d.join("o.plain/a/b.txt")).unwrap(), b"2");
        assert_no_part(&d);
    }

    /// `.part` du dossier effacé (en partie ou en entier) pendant l'écriture, comme par un
    /// autre processus qui l'a cru orphelin (#357) : rien n'est publié, aucun `.part`.
    #[test]
    fn dossier_part_efface_pendant_l_ecriture() {
        let pkg = sample();
        let n = pkg.entries.len();
        for whole in [false, true] {
            let d = scratch(&format!("efface-{whole}"));
            old_output(&d);
            let t = Target::new(Some(&d.join("neuf.plain")), false, &pkg);
            let e = write_package_with(&pkg, &t, ZipLimits::PYTHON, &never, &mut (), &mut |s| {
                let at = if whole { 3 } else { n - 1 };
                if s == Step::WriteDir(at) {
                    let tmp = tracked_part(&d.join("neuf.plain")).unwrap();
                    if whole {
                        discard(&[tmp]);
                    } else {
                        fs::remove_file(tmp.join("version"))?;
                    }
                }
                Ok(())
            })
            .unwrap_err();
            assert_eq!(e.code, "output_error", "entier : {whole}");
            assert!(!d.join("neuf.plain").exists());
            assert_no_part(&d);
        }
    }

    /// Renommage du dossier entier (cible absente) en échec : rien de publié, aucun `.part`.
    #[test]
    fn renommage_du_dossier_entier_en_echec() {
        let pkg = sample();
        let d = scratch("dossier-entier");
        let t = Target::new(Some(&d.join("o.plain")), false, &pkg);
        let e = write_package_with(&pkg, &t, ZipLimits::PYTHON, &never, &mut (), &mut |s| {
            if s == Step::Rename(0) {
                return Err(io::Error::other("panne"));
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(e.code, "output_error");
        assert!(!d.join("o.plain").exists());
        assert_no_part(&d);
        write_package(&pkg, &t, &never).unwrap();
        assert_eq!(fs::read(d.join("o.plain/version")).unwrap(), b"1");
        assert_no_part(&d);
    }

    /// `--plain` vers un `.plain` qui est un fichier : refus avant toute publication, le
    /// `.plain.pk` existant n'est pas remplacé (#357).
    #[test]
    fn plain_fichier_refuse_avant_publication() {
        let pkg = sample();
        let d = scratch("plain-fichier");
        fs::write(d.join("o.plain.pk"), b"ancien paquet").unwrap();
        fs::write(d.join("o.plain"), b"fichier").unwrap();
        let t = Target::new(Some(&d.join("o.plain.pk")), true, &pkg);
        let e = write_package(&pkg, &t, &never).unwrap_err();
        assert_eq!(e.code, "output_error");
        assert_eq!(fs::read(d.join("o.plain.pk")).unwrap(), b"ancien paquet");
        assert_eq!(fs::read(d.join("o.plain")).unwrap(), b"fichier");
        assert_no_part(&d);
    }

    /// Source qui relit normalement, sauf à l'appel n° `fail_at` : la moitié du premier tour
    /// est transmise, puis l'entrée est déclarée modifiée.
    struct Flaky<S> {
        inner: S,
        calls: usize,
        fail_at: usize,
    }

    impl<S: Source> Source for Flaky<S> {
        fn copy(
            &mut self,
            name: &str,
            size: u64,
            crc: u32,
            cancelled: &dyn Fn() -> bool,
            sink: Sink<'_>,
        ) -> Result<(), ZipStop> {
            self.calls += 1;
            if self.calls != self.fail_at {
                return self.inner.copy(name, size, crc, cancelled, sink);
            }
            self.inner
                .copy(name, size, crc, cancelled, &mut |c: &[u8]| {
                    sink(&c[..c.len() / 2])?;
                    Err(ZipStop::Source(ConvError::archive_unreadable("modifiee")))
                })
        }
    }

    /// Relecture en échec au milieu d'une entrée (#402), au `.plain.pk` comme au dossier
    /// `.plain` : `archive_unreadable`, ancienne sortie intacte, aucun `.part`.
    #[test]
    fn relecture_en_echec_au_pk_et_au_dossier() {
        for (what, out, plain, fail_at) in [
            ("pk", "o.plain.pk", false, 1),
            ("pk avec dossier", "o.plain.pk", true, 1),
            ("dossier apres pk", "o.plain.pk", true, 2),
            ("dossier seul", "o.plain", false, 1),
        ] {
            let (archive, pkg) = streamed_sample(&[0xFFu8; 3000]);
            let d = scratch(&format!("relecture-{fail_at}-{plain}-{}", out.len()));
            old_output(&d);
            let t = Target::new(Some(&d.join(out)), plain, &pkg);
            let src = Flaky {
                inner: archive,
                calls: 0,
                fail_at,
            };
            let e = write_any(
                &pkg,
                src,
                &t,
                ZipLimits::PYTHON,
                &never,
                &mut (),
                &mut |_| Ok(()),
            )
            .unwrap_err();
            assert_eq!(e.code, "archive_unreadable", "{what}");
            assert_eq!(fs::read(d.join("o.plain.pk")).unwrap(), b"ancien paquet");
            assert_eq!(fs::read(d.join("o.plain/version")).unwrap(), b"ancien");
            assert_no_part(&d);
        }
    }

    /// Source qui signale sa fermeture.
    struct Probe<'a, S> {
        inner: S,
        closed: &'a Cell<bool>,
    }

    impl<S> Drop for Probe<'_, S> {
        fn drop(&mut self) {
            self.closed.set(true);
        }
    }

    impl<S: Source> Source for Probe<'_, S> {
        fn copy(
            &mut self,
            name: &str,
            size: u64,
            crc: u32,
            cancelled: &dyn Fn() -> bool,
            sink: Sink<'_>,
        ) -> Result<(), ZipStop> {
            self.inner.copy(name, size, crc, cancelled, sink)
        }
    }

    /// L'archive d'entrée est fermée avant le premier renommage de publication (#402).
    #[test]
    fn archive_fermee_avant_publication() {
        let (archive, pkg) = streamed_sample(&[0xFFu8; 3000]);
        let d = scratch("fermee");
        old_output(&d);
        let t = Target::new(Some(&d.join("o.plain.pk")), true, &pkg);
        let closed = Cell::new(false);
        let renames = Cell::new(0);
        let src = Probe {
            inner: archive,
            closed: &closed,
        };
        write_any(
            &pkg,
            src,
            &t,
            ZipLimits::PYTHON,
            &never,
            &mut (),
            &mut |s| {
                match s {
                    Step::Rename(_) => {
                        assert!(closed.get(), "archive encore ouverte à la publication");
                        renames.set(renames.get() + 1);
                    }
                    _ => assert!(!closed.get(), "archive fermée trop tôt ({s:?})"),
                }
                Ok(())
            },
        )
        .unwrap();
        assert!(renames.get() > 1);
        assert_eq!(read_pk(&d.join("o.plain.pk")), sample().entries);
    }

    /// MP3 de plusieurs tours (#402) : mêmes octets qu'en mémoire, au `.plain.pk` comme au
    /// dossier ; l'annulation est consultée entre les tours, et une annulation à n'importe
    /// quel appel laisse l'ancienne sortie intacte, sans `.part`.
    #[test]
    fn copie_par_tours_et_annulation() {
        let mp3: Vec<u8> = (0..2 * COPY_CHUNK + 1000)
            .map(|i| (i % 251) as u8)
            .collect();
        let mut mem = sample();
        for (n, data) in &mut mem.entries {
            if n == "sounds/s.mp3" {
                *data = mp3.clone();
            }
        }
        let d = scratch("tours");
        let t = Target::new(Some(&d.join("o.plain.pk")), true, &mem);
        let (archive, pkg) = streamed_sample(&mp3);
        let calls = Cell::new(0usize);
        let count = || {
            calls.set(calls.get() + 1);
            false
        };
        write_any(
            &pkg,
            archive,
            &t,
            ZipLimits::PYTHON,
            &count,
            &mut (),
            &mut |_| Ok(()),
        )
        .unwrap();
        assert_eq!(read_pk(&d.join("o.plain.pk")), mem.entries);
        assert_eq!(fs::read(d.join("o.plain/sounds/s.mp3")).unwrap(), mp3);
        // Deux tours après le premier, au `.plain.pk` puis au dossier.
        let total = calls.get();
        assert_eq!(total, 2 * mem.entries.len() + 1 + 2 * 2);
        for n in 0..total {
            old_output(&d);
            let (archive, _) = streamed_sample(&mp3);
            let calls = Cell::new(0usize);
            let cancel = || {
                calls.set(calls.get() + 1);
                calls.get() > n
            };
            let e = write_any(
                &pkg,
                archive,
                &t,
                ZipLimits::PYTHON,
                &cancel,
                &mut (),
                &mut |_| Ok(()),
            )
            .unwrap_err();
            assert_eq!(e.code, "cancelled", "appel {n}");
            assert_eq!(fs::read(d.join("o.plain.pk")).unwrap(), b"ancien paquet");
            assert_no_part(&d);
        }
    }

    /// Registre comparé par nom de fichier (#357) : un `.part` de ce processus inscrit sous
    /// une autre écriture du chemin est épargné ; retiré du registre, il est supprimé.
    #[test]
    fn orphelin_du_processus_compare_par_nom() {
        let d = scratch("par-nom");
        let name = format!("o.plain.pk.{}-777777.part", std::process::id());
        fs::write(d.join(&name), b"en cours").unwrap();
        fs::create_dir(d.join("x")).unwrap();
        let other = d.join("x").join("..").join(&name);
        track(&other);
        clean_orphans(&d.join("o.plain.pk"));
        assert!(d.join(&name).exists(), "écriture en cours supprimée");
        untrack(std::slice::from_ref(&other));
        clean_orphans(&d.join("o.plain.pk"));
        assert!(!d.join(&name).exists());
    }

    /// Échec au milieu du `.plain.pk` : le tampon est jeté sans être écrit dans le `.part`.
    #[test]
    fn tampon_jete_sur_echec() {
        let pkg = big_package();
        let d = scratch("tampon");
        let t = Target::new(Some(&d.join("o.plain.pk")), false, &pkg);
        let after = Cell::new(0usize);
        let failed = Cell::new(false);
        let e = write_package_with(&pkg, &t, ZipLimits::PYTHON, &never, &mut (), &mut |s| {
            match s {
                Step::WritePk(3) => {
                    failed.set(true);
                    return Err(io::Error::other("panne"));
                }
                Step::FilePk(_) if failed.get() => after.set(after.get() + 1),
                _ => {}
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(e.code, "output_error");
        assert_eq!(after.get(), 0, "tampon vidé après l'échec");
        assert_no_part(&d);
    }

    /// Panne de sortie et archive d'entrée illisible : la cause première (`archive_unreadable`)
    /// l'emporte sur `output_error`, au `.plain.pk` comme au dossier `.plain`.
    #[test]
    fn archive_illisible_l_emporte_sur_une_panne_de_sortie() {
        let (_, pkg) = streamed_sample(&[0xFFu8; 3000]);
        for (what, out, plain, at) in [
            ("pk", "o.plain.pk", false, Step::CreatePk),
            ("dossier", "o.plain", false, Step::CreateDir),
            ("dossier apres pk", "o.plain.pk", true, Step::WritePk(0)),
        ] {
            let d = scratch(&format!("cause-{what}").replace(' ', "-"));
            old_output(&d);
            let t = Target::new(Some(&d.join(out)), plain, &pkg);
            let mut hook = |s: Step| {
                if s == at {
                    Err(io::Error::other("disque plein"))
                } else {
                    Ok(())
                }
            };
            // Sans archive illisible : la panne de sortie reste `output_error`.
            let (good, _) = streamed_sample(&[0xFFu8; 3000]);
            let e = write_any(
                &pkg,
                good,
                &t,
                ZipLimits::PYTHON,
                &never,
                &mut (),
                &mut hook,
            )
            .unwrap_err();
            assert_eq!(e.code, "output_error", "{what}");
            // Archive modifiée : `archive_unreadable`.
            let (bad, _) = streamed_sample(&[0xFEu8; 3000]);
            let e = write_any(&pkg, bad, &t, ZipLimits::PYTHON, &never, &mut (), &mut hook)
                .unwrap_err();
            assert_eq!(e.code, "archive_unreadable", "{what}");
            assert_eq!(fs::read(d.join("o.plain.pk")).unwrap(), b"ancien paquet");
            assert_eq!(fs::read(d.join("o.plain/version")).unwrap(), b"ancien");
            assert_no_part(&d);
        }
    }

    /// Une panique qui se déroule (bibliothèque) ne laisse ni `.part` ni inscription au registre.
    #[test]
    fn panique_qui_se_deroule_desinscrit_les_part() {
        let pkg = sample();
        let d = scratch("panique-deroulee");
        let t = Target::new(Some(&d.join("o.plain.pk")), true, &pkg);
        let mut hook = |s: Step| {
            if s == Step::WriteDir(1) {
                panic!("panique de test");
            }
            Ok(())
        };
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            write_package_with(&pkg, &t, ZipLimits::PYTHON, &never, &mut (), &mut hook)
        }));
        assert!(r.is_err());
        assert_no_part(&d);
        assert!(
            pending().iter().all(|p| !p.starts_with(&*d)),
            "registre : {:?}",
            pending()
        );
        assert_eq!(fs::read_dir(&*d).unwrap().count(), 0);
    }

    /// Mesure (#416) : coût du `sync_all` par fichier du `.plain`. `cargo test -p telmi2flam
    /// --release mesure_sync_all -- --ignored --nocapture` ; `T2F_MESURE_DIR` choisit le disque.
    #[test]
    #[ignore = "mesure, pas un test"]
    fn mesure_sync_all() {
        use std::time::Instant;
        let n: usize = std::env::var("T2F_MESURE_FICHIERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(500);
        let base = std::env::var_os("T2F_MESURE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let entries: Vec<(String, Vec<u8>)> = (0..n)
            .map(|i| {
                let size = if i % 5 == 0 { 200_000 } else { 4_000 };
                (format!("d{}/f{i}.bin", i % 7), vec![(i % 251) as u8; size])
            })
            .collect();
        // Variantes brutes (sans la crate) : sans sync, par fichier, final séquentiel, final sur 8 fils.
        for round in 0..3 {
            for mode in ["aucun", "par fichier", "final sequentiel", "final 8 fils"] {
                let d = base.join(format!("t2f-mesure-v-{}-{round}", std::process::id()));
                let _ = fs::remove_dir_all(&d);
                fs::create_dir_all(&d).unwrap();
                let t0 = Instant::now();
                let mut files = Vec::new();
                for (i, (_, data)) in entries.iter().enumerate() {
                    let path = d.join(format!("f{i}.bin"));
                    let mut f = fs::File::create(&path).unwrap();
                    f.write_all(data).unwrap();
                    if mode == "par fichier" {
                        f.sync_all().unwrap();
                    } else if mode.starts_with("final") {
                        files.push(f);
                    }
                }
                if mode == "final sequentiel" {
                    for f in &files {
                        f.sync_all().unwrap();
                    }
                } else if mode == "final 8 fils" {
                    let chunks: Vec<&[fs::File]> = files.chunks(files.len().div_ceil(8)).collect();
                    std::thread::scope(|sc| {
                        for c in chunks {
                            sc.spawn(move || c.iter().for_each(|f| f.sync_all().unwrap()));
                        }
                    });
                }
                drop(files);
                eprintln!(
                    "MESURE-V {mode} : {n} fichiers : {:.0} ms",
                    t0.elapsed().as_secs_f64() * 1000.0
                );
                let _ = fs::remove_dir_all(&d);
            }
        }
    }
}
