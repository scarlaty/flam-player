//! Vignettes des histoires installées (#80) : `<cache>/library/thumbs/<nom>`, référencées par
//! `LibraryEntry::thumb`.
//!
//! Une vignette n'est **jamais** indispensable : préparation, écriture ou lecture en échec
//! donnent « pas de vignette » (avertissement), jamais une installation ou une désinstallation
//! en échec ; le front affiche alors l'illustration par défaut.
//!
//! - **Préparation** ([`prepare`], hors verrou, sans réseau) : vignette déjà téléchargée dans le
//!   cache du catalogue d'origine, sinon, pour un pack Lunii `.lunii.pk` (histoire STUdio, #554),
//!   son `_thumbnail.png` d'origine, sinon `img/thumbnail.lif` du pack produit (`.plain.pk` ou
//!   `.lunii.pk`), réencodée en PNG. Octets reconnus par leur signature (PNG, JPEG, GIF, WebP),
//!   [`MAX_BYTES`] au plus.
//! - **Écriture** ([`store`], sous le verrou `library/.lock`, après les refus de l'installation
//!   et avant l'index) : nom unique par publication (`<uuid>-<version>.<installed_at>.<ext>`),
//!   écriture atomique ; la vignette d'une installation précédente n'est jamais écrasée.
//! - **Réconciliation** ([`sweep`], sous le verrou, index faisant foi et aucun index mis de côté
//!   `library.json.*.bak`, qui reste restaurable à la main : #97) : tout fichier de `thumbs/` que
//!   l'index ne référence pas est supprimé (vignette d'une publication défaite, d'une ancienne
//!   version, d'une histoire désinstallée, temporaires).

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::catalog::{cached_thumb_path, sniff_image};
use crate::library::{file_stem, remove_if_exists, LibraryEntry, StoryFormat};
use crate::story::StoryEntry;
use crate::thumbnail;
use crate::{storage, AppError, AppResult};

/// Sous-dossier des vignettes, dans `library/`.
pub const THUMBS_DIR: &str = "thumbs";
/// Taille maximale d'une vignette conservée.
pub const MAX_BYTES: u64 = 256 * 1024;
/// Côté maximal d'une vignette extraite d'un `.plain.pk`.
const THUMB_DIM: u32 = 192;
/// Vignette dans un `.plain.pk`.
const LIF_NAME: &str = "img/thumbnail.lif";
/// Taille maximale du `img/thumbnail.lif` lu dans le `.plain.pk`.
const MAX_LIF_BYTES: u64 = 4 * 1024 * 1024;
/// Vignette d'origine d'un pack Lunii (`install::studio`).
const PACK_PNG_NAME: &str = "_thumbnail.png";
/// Taille maximale du `_thumbnail.png` lu dans le `.lunii.pk`.
const MAX_PACK_PNG_BYTES: u64 = 4 * 1024 * 1024;

/// Vignette prête à écrire : octets d'image reconnus et extension correspondante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    bytes: Vec<u8>,
    ext: &'static str,
}

impl Prepared {
    /// Octets reconnus comme image (signature) et d'au plus [`MAX_BYTES`], sinon `None`.
    pub fn from_bytes(bytes: Vec<u8>) -> Option<Self> {
        if bytes.len() as u64 > MAX_BYTES {
            return None;
        }
        let ext = match sniff_image(&bytes)? {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            _ => return None,
        };
        Some(Self { bytes, ext })
    }
}

/// Vignette de l'histoire `source` qui vient d'être produite dans `staged` : celle du cache du
/// catalogue si elle y est, sinon celle du pack (`_thumbnail.png` d'un `.lunii.pk`, puis
/// `img/thumbnail.lif`). `None` (avertissement) si aucune n'est exploitable. Ne fait jamais
/// d'accès réseau.
pub fn prepare(cache_root: &Path, source: &StoryEntry, staged: &Path) -> Option<Prepared> {
    if let Some(prepared) = from_catalog_cache(cache_root, source) {
        return Some(prepared);
    }
    let lunii = staged
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(StoryFormat::of_file)
        == Some(StoryFormat::Lunii);
    if lunii {
        match from_pack_png(staged) {
            Ok(prepared) => return Some(prepared),
            Err(reason) => tracing::info!(
                "histoire {} : vignette d'origine inutilisable ({reason}), essai de {LIF_NAME}",
                source.uuid
            ),
        }
    }
    match from_plain_pk(staged) {
        Ok(prepared) => Some(prepared),
        Err(reason) => {
            tracing::info!(
                "histoire {} installée sans vignette : {reason}",
                source.uuid
            );
            None
        }
    }
}

/// Vignette déjà téléchargée par `catalog_thumbnail` pour une entrée de catalogue.
fn from_catalog_cache(cache_root: &Path, source: &StoryEntry) -> Option<Prepared> {
    let remote = |u: &str| {
        let u = u.to_ascii_lowercase();
        u.starts_with("http://") || u.starts_with("https://")
    };
    let thumb_url = source.thumb_url.as_deref()?;
    if !remote(&source.download_url) || !remote(thumb_url) {
        return None;
    }
    let path = cached_thumb_path(cache_root, &source.source_id, thumb_url)?;
    let bytes = read_bounded(&path, MAX_BYTES).ok()?;
    Prepared::from_bytes(bytes)
}

/// Entrée `name` d'un pack (zip), lue avec la borne `max` (déclarée puis lue).
fn read_pack_entry(path: &Path, name: &str, max: u64) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("ouverture impossible : {e}"))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|e| format!("archive invalide : {e}"))?;
    let entry = zip.by_name(name).map_err(|_| format!("pas de {name}"))?;
    if entry.size() > max {
        return Err(format!("{name} trop volumineux"));
    }
    let mut bytes = Vec::new();
    entry
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("lecture de {name} impossible : {e}"))?;
    if bytes.len() as u64 > max {
        return Err(format!("{name} trop volumineux"));
    }
    Ok(bytes)
}

/// `_thumbnail.png` d'un `.lunii.pk`, décodé (bornes de `thumbnail::decode_png`), réduit et
/// réencodé en PNG.
fn from_pack_png(path: &Path) -> Result<Prepared, String> {
    let png = read_pack_entry(path, PACK_PNG_NAME, MAX_PACK_PNG_BYTES)?;
    let image = thumbnail::decode_png(&png).map_err(|e| format!("{PACK_PNG_NAME} : {e}"))?;
    let png =
        thumbnail::encode_png(&thumbnail::fit(image, THUMB_DIM)).map_err(|e| e.to_string())?;
    Prepared::from_bytes(png).ok_or_else(|| "vignette réencodée trop volumineuse".into())
}

/// `img/thumbnail.lif` du pack (`.plain.pk` ou `.lunii.pk`), décodé, réduit et encodé en PNG.
fn from_plain_pk(path: &Path) -> Result<Prepared, String> {
    let lif = read_pack_entry(path, LIF_NAME, MAX_LIF_BYTES)?;
    let image = telmi2flam::lif::decode(&lif).map_err(|e| format!("{LIF_NAME} : {e}"))?;
    let png =
        thumbnail::encode_png(&thumbnail::fit(image, THUMB_DIM)).map_err(|e| e.to_string())?;
    Prepared::from_bytes(png).ok_or_else(|| "vignette réencodée trop volumineuse".into())
}

/// Lit au plus `limit` octets ; plus gros : erreur.
fn read_bounded(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(std::io::Error::other("vignette trop volumineuse"));
    }
    Ok(bytes)
}

/// Nom de la vignette de la publication `file` faite à `now` : `<uuid>-<version>.<now>.<ext>`
/// pour un `.plain.pk`, `<uuid>-<version>.lunii.<now>.<ext>` pour un `.lunii.pk` (deux
/// publications de formats différents dans la même seconde n'écrivent jamais le même fichier).
fn thumb_name(file: &str, now: u64, ext: &str) -> String {
    let stem = match StoryFormat::of_file(file) {
        Some(StoryFormat::Lunii) => file.strip_suffix(".pk").unwrap_or(file),
        _ => file_stem(file).unwrap_or(file),
    };
    format!("{stem}.{now}.{ext}")
}

/// Chemin d'une vignette : nom simple, directement sous `library/thumbs/`, sinon `None`. `/` et
/// `\` sont refusés sur la chaîne elle-même, quelle que soit la plateforme (un index écrit sous
/// Windows peut être relu ailleurs).
fn checked(library_root: &Path, name: &str) -> Option<PathBuf> {
    let dir = library_root.join(THUMBS_DIR);
    let path = dir.join(name);
    let simple = !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\'])
        && path.parent() == Some(dir.as_path())
        && path.file_name().and_then(|n| n.to_str()) == Some(name);
    simple.then_some(path)
}

/// Écrit la vignette de la publication `file` (verrou de la bibliothèque tenu). Renvoie le nom à
/// mettre dans `LibraryEntry::thumb`, ou `None` (avertissement) si l'écriture échoue.
///
/// Toute erreur de l'installation **après** cet appel (neutralisation des marqueurs non
/// jugeables, marqueur, renommages, index) laisse la vignette écrite orpheline : aucune entrée ne
/// la référence, la réconciliation suivante ([`sweep`]) la supprime.
pub fn store(library_root: &Path, file: &str, now: u64, prepared: &Prepared) -> Option<String> {
    store_with(library_root, file, now, prepared, &|path, bytes| {
        storage::write_atomic(path, bytes)
    })
}

/// [`store`] avec une fonction d'écriture injectable (tests d'injection de panne).
pub(crate) fn store_with(
    library_root: &Path,
    file: &str,
    now: u64,
    prepared: &Prepared,
    write: &dyn Fn(&Path, &[u8]) -> std::io::Result<()>,
) -> Option<String> {
    let name = thumb_name(file, now, prepared.ext);
    let path = checked(library_root, &name)?;
    match write(&path, &prepared.bytes) {
        Ok(()) => Some(name),
        Err(err) => {
            tracing::warn!("vignette {} non écrite : {err}", path.display());
            None
        }
    }
}

/// Supprime de `library/thumbs/` tout fichier que `stories` ne référence pas (verrou tenu,
/// index faisant foi, aucun index mis de côté, `library/` lisible). Échecs : avertissement,
/// retenté à la réconciliation suivante.
///
/// Une réinstallation sans vignette exploitable (préparation ou écriture en échec) remplace
/// l'entrée par une entrée `thumb: None` : la vignette de l'installation précédente n'est plus
/// référencée et part ici ; l'histoire retombe sur la vignette de sa source d'origine, puis sur
/// l'illustration par défaut.
pub fn sweep(library_root: &Path, stories: &[LibraryEntry]) {
    // Seul un `library/` marqué comme créé par l'application est balayé (#570).
    if !crate::cache::is_owned(library_root) {
        tracing::warn!("bibliothèque sans marqueur de l'application : vignettes non balayées");
        return;
    }
    let Ok(read) = std::fs::read_dir(library_root.join(THUMBS_DIR)) else {
        return;
    };
    for item in read.flatten() {
        if !item.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let name = item.file_name().to_string_lossy().into_owned();
        if stories.iter().any(|e| e.thumb.as_deref() == Some(&*name)) {
            continue;
        }
        if let Err(err) = remove_if_exists(&item.path()) {
            tracing::warn!("vignette orpheline {name} non supprimée : {err}");
        }
    }
}

/// URL `data:` de la vignette de `entry` ; `not_found` si elle n'en a pas ou si le fichier a
/// disparu ou n'est plus une image (le front affiche alors l'illustration par défaut).
pub fn data_url(library_root: &Path, entry: &LibraryEntry) -> AppResult<String> {
    let missing = || AppError::NotFound(format!("pas de vignette pour {}", entry.uuid));
    let path = entry
        .thumb
        .as_deref()
        .and_then(|name| checked(library_root, name))
        .ok_or_else(missing)?;
    let bytes = read_bounded(&path, MAX_BYTES).map_err(|_| missing())?;
    let mime = sniff_image(&bytes).ok_or_else(missing)?;
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{mime};base64,{encoded}"))
}

/// Vignette conservée de l'histoire installée `uuid` (URL `data:`) ; `not_found` sans vignette.
#[tauri::command]
pub async fn library_thumbnail(
    service: tauri::State<'_, crate::install::InstallService>,
    uuid: String,
) -> AppResult<String> {
    service.library_thumbnail(uuid).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::story;
    use crate::library::Library;
    use std::io::Write;

    /// LIF 1x1 valide (même charge que les tests de `folder`).
    fn lif() -> Vec<u8> {
        let mut v = b"liff".to_vec();
        v.extend_from_slice(&1u32.to_be_bytes());
        v.extend_from_slice(&1u32.to_be_bytes());
        v.push(0xA2);
        v.extend_from_slice(&[0xFF, 0x1F, 0xF8, 255]);
        v.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
        v
    }

    /// `.plain.pk` préparé dans `.staging/`, avec `img/thumbnail.lif` = `thumb` s'il est donné.
    fn staged_pk(lib: &Library, name: &str, thumb: Option<&[u8]>) -> PathBuf {
        let path = lib.staging_dir().unwrap().join(name);
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (file, data) in [("info.plain", b"Titre".as_slice()), ("main.lua", b"")] {
            zip.start_file(file, options).unwrap();
            zip.write_all(data).unwrap();
        }
        if let Some(thumb) = thumb {
            zip.start_file(LIF_NAME, options).unwrap();
            zip.write_all(thumb).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    fn png() -> Vec<u8> {
        thumbnail::encode_png(&thumbnail::RgbaImage {
            width: 2,
            height: 2,
            rgba: [1, 2, 3, 255].repeat(4),
        })
        .unwrap()
    }

    fn thumbs(root: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(root.join("library").join(THUMBS_DIR))
            .map(|r| {
                r.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// Pack Lunii `.lunii.pk` préparé dans `.staging/` (#554), avec `_thumbnail.png` et
    /// `img/thumbnail.lif` s'ils sont donnés.
    fn staged_lunii(lib: &Library, name: &str, png: Option<&[u8]>, lif: Option<&[u8]>) -> PathBuf {
        let path = lib.staging_dir().unwrap().join(name);
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("ni", options).unwrap();
        zip.write_all(b"ni").unwrap();
        for (file, data) in [(PACK_PNG_NAME, png), (LIF_NAME, lif)] {
            if let Some(data) = data {
                zip.start_file(file, options).unwrap();
                zip.write_all(data).unwrap();
            }
        }
        zip.finish().unwrap();
        path
    }

    fn studio(uuid: &str, version: Option<u32>) -> StoryEntry {
        let mut s = story(uuid, version);
        s.kind = crate::story::StoryKind::StudioZip;
        s
    }

    /// Pack Lunii : vignette d'origine `_thumbnail.png`, sinon `img/thumbnail.lif` ; une
    /// vignette illisible n'empêche jamais l'installation (#529).
    #[test]
    fn vignette_d_un_pack_lunii_png_puis_lif_puis_aucune() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let s = staged_lunii(&lib, "t.lunii.pk", Some(&png()), Some(&lif()));
        let entry = install(&lib, dir.path(), &studio("abc", Some(3)), &s, 7);
        assert_eq!(entry.thumb.as_deref(), Some("abc-3.lunii.7.png"));
        assert!(lib
            .thumbnail("abc")
            .unwrap()
            .starts_with("data:image/png;base64,"));

        // `_thumbnail.png` illisible : repli sur le LIF.
        let s = staged_lunii(
            &lib,
            "u.lunii.pk",
            Some(b"\x89PNG pas une image"),
            Some(&lif()),
        );
        let entry = install(&lib, dir.path(), &studio("def", Some(1)), &s, 8);
        assert_eq!(entry.thumb.as_deref(), Some("def-1.lunii.8.png"));

        // Aucune vignette exploitable : installée sans vignette.
        let s = staged_lunii(&lib, "v.lunii.pk", Some(b"illisible"), Some(b"pas un lif"));
        let entry = install(&lib, dir.path(), &studio("ghi", Some(1)), &s, 9);
        assert_eq!(entry.thumb, None);
        assert_eq!(lib.thumbnail("ghi").unwrap_err().kind(), "not_found");
        assert_eq!(
            thumbs(dir.path()),
            ["abc-3.lunii.7.png", "def-1.lunii.8.png"]
        );
    }

    #[test]
    fn noms_de_vignettes_distincts_selon_le_format() {
        assert_eq!(thumb_name("abc-2.plain.pk", 5, "png"), "abc-2.5.png");
        assert_eq!(thumb_name("abc-2.lunii.pk", 5, "png"), "abc-2.lunii.5.png");
    }

    /// Installe comme le pipeline : préparation puis publication.
    fn install(
        lib: &Library,
        cache: &Path,
        source: &StoryEntry,
        staged: &Path,
        now: u64,
    ) -> LibraryEntry {
        let prepared = prepare(cache, source, staged);
        lib.install_thumbed(staged, source, None, now, prepared.as_ref(), None)
            .unwrap()
    }

    #[test]
    fn installation_avec_vignette_du_plain_pk_puis_desinstallation() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let s = staged_pk(&lib, "t.plain.pk", Some(&lif()));
        let entry = install(&lib, dir.path(), &story("abc", Some(3)), &s, 7);
        assert_eq!(entry.thumb.as_deref(), Some("abc-3.7.png"));
        assert_eq!(thumbs(dir.path()), ["abc-3.7.png"]);
        assert_eq!(lib.list().unwrap()[0].thumb, entry.thumb);
        assert!(lib
            .thumbnail("abc")
            .unwrap()
            .starts_with("data:image/png;base64,"));

        lib.remove("abc").unwrap();
        assert!(thumbs(dir.path()).is_empty());
        assert!(matches!(lib.thumbnail("abc"), Err(AppError::NotFound(_))));
    }

    #[test]
    fn installation_sans_vignette_ou_vignette_illisible() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let s = staged_pk(&lib, "a.plain.pk", None);
        let entry = install(&lib, dir.path(), &story("abc", Some(1)), &s, 1);
        assert_eq!(entry.thumb, None);
        assert!(matches!(lib.thumbnail("abc"), Err(AppError::NotFound(_))));

        let s = staged_pk(&lib, "b.plain.pk", Some(b"pas un lif"));
        let entry = install(&lib, dir.path(), &story("def", Some(1)), &s, 2);
        assert_eq!(entry.thumb, None);
        assert!(thumbs(dir.path()).is_empty());
        assert_eq!(lib.list().unwrap().len(), 2);
    }

    #[test]
    fn vignette_du_cache_du_catalogue_prioritaire_et_bornee() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let url = "https://img.example.test/abc.jpg";
        let mut source = story("abc", Some(2));
        source.thumb_url = Some(url.into());
        let cached = cached_thumb_path(dir.path(), &source.source_id, url).unwrap();
        std::fs::create_dir_all(cached.parent().unwrap()).unwrap();

        // JPEG en cache : copié tel quel.
        std::fs::write(&cached, b"\xFF\xD8\xFF\xE0jpeg").unwrap();
        let s = staged_pk(&lib, "a.plain.pk", Some(&lif()));
        let entry = install(&lib, dir.path(), &source, &s, 5);
        assert_eq!(entry.thumb.as_deref(), Some("abc-2.5.jpg"));
        assert!(lib
            .thumbnail("abc")
            .unwrap()
            .starts_with("data:image/jpeg;"));

        // Trop grosse ou non reconnue (SVG) : ignorée, repli sur le `.plain.pk`.
        let mut huge = png();
        huge.resize(MAX_BYTES as usize + 1, 0);
        for bad in [huge.clone(), b"<svg/>".to_vec()] {
            std::fs::write(&cached, &bad).unwrap();
            let s = staged_pk(&lib, "b.plain.pk", Some(&lif()));
            assert_eq!(prepare(dir.path(), &source, &s).map(|p| p.ext), Some("png"));
        }
        assert_eq!(Prepared::from_bytes(huge), None);
        assert!(Prepared::from_bytes(png()).is_some());
    }

    #[test]
    fn echec_d_ecriture_de_la_vignette_n_empeche_pas_l_installation() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let s = staged_pk(&lib, "t.plain.pk", Some(&lif()));
        // Injection : `library/thumbs` est un fichier, le dossier ne peut pas être créé.
        std::fs::write(dir.path().join("library").join(THUMBS_DIR), b"x").unwrap();
        let entry = install(&lib, dir.path(), &story("abc", Some(1)), &s, 1);
        assert_eq!(entry.thumb, None);
        assert!(dir.path().join("library/abc-1.plain.pk").is_file());
        assert_eq!(lib.list().unwrap(), vec![entry]);

        // Fonction d'écriture en échec : `None`.
        let prepared = Prepared::from_bytes(png()).unwrap();
        let failing = |_: &Path, _: &[u8]| Err(std::io::Error::other("disque plein"));
        assert_eq!(
            store_with(dir.path(), "abc-1.plain.pk", 1, &prepared, &failing),
            None
        );
    }

    #[test]
    fn mise_a_jour_puis_orphelines_reconciliees() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let s = staged_pk(&lib, "a.plain.pk", Some(&lif()));
        install(&lib, dir.path(), &story("abc", Some(1)), &s, 1);
        // Orpheline (publication défaite) et temporaire d'une écriture interrompue.
        let thumbs_dir = dir.path().join("library").join(THUMBS_DIR);
        std::fs::write(thumbs_dir.join("zzz-1.9.png"), png()).unwrap();
        std::fs::write(thumbs_dir.join(".abc-2.3.png.1-0.tmp"), b"x").unwrap();

        // Nouvelle version : l'ancienne vignette et les orphelines partent.
        let s = staged_pk(&lib, "b.plain.pk", Some(&lif()));
        let entry = install(&lib, dir.path(), &story("abc", Some(2)), &s, 3);
        assert_eq!(thumbs(dir.path()), [entry.thumb.unwrap()]);
    }

    #[test]
    fn le_balayage_ne_touche_pas_a_une_bibliotheque_sans_marqueur() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library");
        let thumbs_dir = library.join(THUMBS_DIR);
        std::fs::create_dir_all(&thumbs_dir).unwrap();
        std::fs::write(thumbs_dir.join("photo.png"), png()).unwrap();

        // Homonyme sans marqueur (#570) : rien n'est supprimé.
        sweep(&library, &[]);
        assert_eq!(thumbs(dir.path()), ["photo.png"]);

        // Le même dossier marqué par l'application : l'orpheline part.
        std::fs::write(library.join(crate::cache::MARKER), b"").unwrap();
        sweep(&library, &[]);
        assert!(thumbs(dir.path()).is_empty());
    }

    #[test]
    fn vignette_disparue_donne_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let s = staged_pk(&lib, "a.plain.pk", Some(&lif()));
        let entry = install(&lib, dir.path(), &story("abc", Some(1)), &s, 1);
        let name = entry.thumb.unwrap();
        std::fs::remove_file(dir.path().join("library").join(THUMBS_DIR).join(name)).unwrap();
        assert!(matches!(lib.thumbnail("abc"), Err(AppError::NotFound(_))));
        assert_eq!(lib.list().unwrap().len(), 1);
    }

    #[test]
    fn noms_hors_du_dossier_refuses() {
        let root = Path::new("lib");
        assert!(checked(root, "a.png").is_some());
        // `\` est refusé sur toutes les plateformes, pas seulement là où il sépare les chemins.
        for bad in [
            "", ".cache", "../a.png", "a/b.png", "/a.png", "a\\b.png", "a\\", "\\a.png",
        ] {
            assert!(checked(root, bad).is_none(), "{bad:?}");
        }
    }

    /// Injection de panne (#97) : index corrompu mis de côté (`library.json.*.bak`). Les
    /// installations et désinstallations qui suivent, sur un nouvel index valide, ne réconcilient
    /// pas : une fois le `.bak` réparé et remis en place à la main, les vignettes qu'il référence
    /// existent. Le `.bak` parti, la réconciliation reprend.
    #[test]
    fn index_mis_de_cote_restaure_garde_ses_vignettes() {
        use crate::library::INDEX_FILE;
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let library = dir.path().join("library");
        let s = staged_pk(&lib, "a.plain.pk", Some(&lif()));
        let old_thumb = install(&lib, dir.path(), &story("abc", Some(1)), &s, 1)
            .thumb
            .unwrap();

        // Index corrompu (fin abîmée) : mis de côté à l'installation suivante, qui ne réconcilie
        // pas.
        let index = library.join(INDEX_FILE);
        let good = std::fs::read(&index).unwrap();
        let mut damaged = good.clone();
        damaged.extend_from_slice(b"\0\0abime");
        std::fs::write(&index, &damaged).unwrap();
        let s = staged_pk(&lib, "b.plain.pk", Some(&lif()));
        install(&lib, dir.path(), &story("def", Some(1)), &s, 2);
        let bak = std::fs::read_dir(&library)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| {
                let name = p.file_name().unwrap().to_string_lossy();
                name.starts_with("library.json.") && name.ends_with(".bak")
            })
            .expect("index mis de côté");

        // Nouvel index valide, `.bak` toujours là : ni l'installation ni la désinstallation ne
        // suppriment la vignette de l'ancien index.
        let s = staged_pk(&lib, "c.plain.pk", Some(&lif()));
        install(&lib, dir.path(), &story("ghi", Some(1)), &s, 3);
        lib.remove("def").unwrap();
        assert!(thumbs(dir.path()).contains(&old_thumb));

        // Restauration manuelle : le `.bak` réparé remplace l'index, l'entrée retrouve sa vignette.
        assert_eq!(std::fs::read(&bak).unwrap(), damaged);
        std::fs::write(&bak, &good).unwrap();
        std::fs::rename(&bak, &index).unwrap();
        assert!(lib
            .thumbnail("abc")
            .unwrap()
            .starts_with("data:image/png;base64,"));

        // Plus d'index mis de côté : la réconciliation reprend (orphelines de `def` et `ghi`
        // supprimées), la vignette référencée par l'index restauré reste.
        let s = staged_pk(&lib, "d.plain.pk", Some(&lif()));
        let new = install(&lib, dir.path(), &story("jkl", Some(1)), &s, 4);
        assert_eq!(thumbs(dir.path()), [old_thumb, new.thumb.unwrap()]);
    }
}
