//! Compléments d'image automatiques (`docs/creation-dossier.md` § 7, « Image d'une histoire ou
//! d'un chapitre »).
//!
//! # API
//!
//! [`complete_image`] est le point d'entrée du lot de construction : il parcourt la chaîne
//!
//! 1. image du même nom (chemin fourni par l'appelant, [`ImageOrigin::Named`]) ;
//! 2. pochette du fichier audio, extraite par ffmpeg ([`ImageOrigin::Cover`]) ;
//! 3. image générée avec le titre ([`ImageOrigin::TitleCard`], voir [`super::title_card`]) ;
//! 4. image unie de [`ImageRequest::fallback_color`] ([`ImageOrigin::Solid`]) ;
//!
//! et s'arrête à la première qui réussit. Une étape qui échoue (image illisible, pas de
//! pochette, ffmpeg absent ou en échec, titre sans caractère affichable) passe à la suivante :
//! seule l'annulation interrompt la chaîne. Le résultat est un PNG aux dimensions de
//! [`ArtworkKind`] ; rien n'est écrit dans le dossier de l'histoire.
//!
//! # Sécurité (§ 10)
//!
//! - ffmpeg est lancé par [`Verified::run`] (empreinte vérifiée, pas de shell, environnement
//!   réduit, délai) avec des **arguments fixes** ([`cover_args`]) : `-protocol_whitelist file`
//!   avant l'entrée, un seul flux vidéo copié tel quel (`-c:v copy`, aucun décodage ni filtre),
//!   métadonnées, audio, sous-titres et données écartés, sortie bornée par `-fs` (entre deux paquets seulement, la relecture est aussi plafonnée). Le chemin
//!   du fichier source est absolu (il ne peut donc pas être pris pour une option) et le fichier
//!   de sortie est créé **dans le dossier de travail** fourni, sous un nom aléatoire, puis
//!   supprimé : rien n'est écrit à côté de la source.
//! - Les octets de la pochette ou de l'image fournie sont décodés par [`Convert`] ; la valeur
//!   de production, [`convert_in_child`], le fait dans un processus enfant (#263).
//! - Les octets lus sont bornés par [`MAX_INPUT_BYTES`].

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::sync::watch;

use crate::error::{AppError, AppResult};
use crate::ffmpeg::{Limits, Verified};
use crate::podcast::artwork::{
    convert_isolated, Artwork, ArtworkError, ArtworkKind, MAX_INPUT_BYTES,
};

use super::title_card;

/// Délai maximal de l'extraction d'une pochette.
pub const COVER_TIMEOUT: Duration = Duration::from_secs(20);

/// Conversion d'octets d'image en [`Artwork`].
pub type Convert = fn(&[u8], ArtworkKind) -> Result<Artwork, ArtworkError>;

/// Conversion de production : décodage dans un processus enfant (bloquant).
pub fn convert_in_child(bytes: &[u8], kind: ArtworkKind) -> Result<Artwork, ArtworkError> {
    convert_isolated(bytes, kind, None)
}

/// Origine de l'image rendue par [`complete_image`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageOrigin {
    /// Image fournie (même nom) : pas un complément.
    Named,
    /// Pochette du fichier audio.
    Cover,
    /// Image générée avec le titre.
    TitleCard,
    /// Image unie.
    Solid,
}

/// Ce que [`complete_image`] doit compléter.
pub struct ImageRequest<'a> {
    /// Usage : fixe la taille de la sortie.
    pub kind: ArtworkKind,
    /// Titre affiché sur l'image générée.
    pub title: &'a str,
    /// Image du même nom, si l'appelant en a trouvé une.
    pub named_image: Option<&'a Path>,
    /// Fichier audio dont extraire la pochette (chemin absolu).
    pub audio: Option<&'a Path>,
    /// ffmpeg vérifié ([`crate::ffmpeg::Ffmpeg::verify`]) ; absent : étape pochette ignorée.
    pub ffmpeg: Option<&'a Verified>,
    /// Dossier de travail existant, où ffmpeg écrit sa sortie temporaire.
    pub work_dir: &'a Path,
    /// Couleur de l'image unie (par exemple `FALLBACK_COLOR`).
    pub fallback_color: [u8; 3],
    /// Décodage des images (en production : [`convert_in_child`]).
    pub convert: Convert,
}

/// Image obtenue et son origine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedImage {
    pub artwork: Artwork,
    pub origin: ImageOrigin,
}

/// Parcourt la chaîne de compléments d'image (voir l'en-tête du module).
///
/// Erreurs : annulation ([`AppError::Cancelled`]) ; l'échec de l'image unie, qui ne peut venir
/// que de l'encodage PNG.
pub async fn complete_image(
    req: ImageRequest<'_>,
    cancel: &mut watch::Receiver<bool>,
) -> AppResult<CompletedImage> {
    let done = |artwork, origin| Ok(CompletedImage { artwork, origin });
    let check = |cancel: &watch::Receiver<bool>| {
        if *cancel.borrow() {
            Err(AppError::Cancelled)
        } else {
            Ok(())
        }
    };
    check(cancel)?;

    if let Some(path) = req.named_image {
        match read_capped(path).await {
            Ok(Some(bytes)) => {
                if let Some(art) = decode(req.convert, bytes, req.kind).await {
                    return done(art, ImageOrigin::Named);
                }
            }
            Ok(None) => {}
            Err(e) => tracing::debug!("image fournie illisible ({}) : {e}", path.display()),
        }
    }
    check(cancel)?;

    if let (Some(audio), Some(ffmpeg)) = (req.audio, req.ffmpeg) {
        match extract_cover(ffmpeg, audio, req.work_dir, cancel).await {
            Ok(Some(bytes)) => {
                if let Some(art) = decode(req.convert, bytes, req.kind).await {
                    return done(art, ImageOrigin::Cover);
                }
            }
            Ok(None) => {}
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(e) => tracing::debug!("pochette non extraite ({}) : {e}", audio.display()),
        }
    }
    check(cancel)?;

    let (title, kind) = (req.title.to_string(), req.kind);
    let card = tokio::task::spawn_blocking(move || title_card::render(&title, kind)).await;
    match card {
        Ok(Ok(Some(art))) => return done(art, ImageOrigin::TitleCard),
        Ok(Ok(None)) => {}
        Err(e) => tracing::debug!("image du titre interrompue : {e}"),
        Ok(Err(e)) => tracing::debug!("image du titre non générée : {e}"),
    }

    let art = Artwork::fallback(req.kind, req.fallback_color)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    done(art, ImageOrigin::Solid)
}

/// Décode `bytes` hors du thread asynchrone ; `None` si l'image est refusée.
async fn decode(convert: Convert, bytes: Vec<u8>, kind: ArtworkKind) -> Option<Artwork> {
    match tokio::task::spawn_blocking(move || convert(&bytes, kind)).await {
        Ok(Ok(art)) => Some(art),
        Ok(Err(e)) => {
            tracing::debug!("image refusée : {e}");
            None
        }
        Err(e) => {
            tracing::debug!("décodage interrompu : {e}");
            None
        }
    }
}

/// Contenu de `path`, `None` s'il dépasse [`MAX_INPUT_BYTES`] ou est vide.
async fn read_capped(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    // Type vérifié avant l'ouverture : ouvrir un tube nommé bloquerait.
    if !tokio::fs::metadata(path).await?.is_file() {
        return Ok(None);
    }
    let file = tokio::fs::File::open(path).await?;
    let mut bytes = Vec::new();
    file.take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .await?;
    Ok((!bytes.is_empty() && bytes.len() <= MAX_INPUT_BYTES).then_some(bytes))
}

/// Arguments fixes de l'extraction d'une pochette (voir l'en-tête du module). Seuls `input` et
/// `output` varient ; `input` est un chemin absolu.
pub fn cover_args(input: &Path, output: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-protocol_whitelist",
        "file",
        "-probesize",
        "8000000",
        "-i",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    args.push(input.into());
    args.extend(
        [
            "-map",
            "0:v:0",
            "-frames:v",
            "1",
            "-c:v",
            "copy",
            "-an",
            "-sn",
            "-dn",
            "-map_metadata",
            "-1",
            "-fs",
            &MAX_INPUT_BYTES.to_string(),
            "-f",
            "image2pipe",
            "-y",
        ]
        .map(OsString::from),
    );
    args.push(output.into());
    args
}

/// Extrait la pochette de `audio` (MP3 ID3 APIC, M4A, FLAC, OGG...) : les octets de l'image
/// telle qu'elle est intégrée (JPEG ou PNG), à décoder par l'appelant.
///
/// `Ok(None)` : pas de pochette (aucun flux vidéo), fichier illisible pour ffmpeg, ou image
/// vide ou plus grosse que [`MAX_INPUT_BYTES`]. Erreurs : chemin non absolu ou qui n'est pas un
/// fichier, `work_dir` absent, ffmpeg impossible à lancer, délai ([`COVER_TIMEOUT`]) dépassé,
/// annulation. La sortie temporaire est écrite dans `work_dir` et supprimée dans tous les cas.
pub async fn extract_cover(
    ffmpeg: &Verified,
    audio: &Path,
    work_dir: &Path,
    cancel: &mut watch::Receiver<bool>,
) -> AppResult<Option<Vec<u8>>> {
    if !audio.is_absolute() {
        return Err(AppError::Validation(format!(
            "chemin audio non absolu : {}",
            audio.display()
        )));
    }
    if !tokio::fs::metadata(audio).await?.is_file() {
        return Err(AppError::Validation(format!(
            "pas un fichier : {}",
            audio.display()
        )));
    }
    if !work_dir.is_absolute() || !tokio::fs::metadata(work_dir).await?.is_dir() {
        return Err(AppError::Validation(format!(
            "dossier de travail absent ou non absolu : {}",
            work_dir.display()
        )));
    }
    let out = work_dir.join(format!("cover-{}.bin", uuid::Uuid::new_v4().simple()));
    let result = ffmpeg
        .run(&cover_args(audio, &out), Limits::new(COVER_TIMEOUT), cancel)
        .await;
    let bytes = match &result {
        Ok(output) if output.success() => read_capped(&out).await.ok().flatten(),
        Ok(output) => {
            tracing::debug!(
                "ffmpeg sans pochette (code {:?}) : {}",
                output.code,
                output.stderr
            );
            None
        }
        Err(_) => None,
    };
    // Sortie partielle comprise (délai, annulation, échec) ; absente si ffmpeg n'a rien écrit.
    let _ = tokio::fs::remove_file(&out).await;
    result.map(|_| bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    fn plain(bytes: &[u8], kind: ArtworkKind) -> Result<Artwork, ArtworkError> {
        Artwork::convert(bytes, kind)
    }

    fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb(rgb));
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn request<'a>(work: &'a Path, title: &'a str) -> ImageRequest<'a> {
        ImageRequest {
            kind: ArtworkKind::Episode,
            title,
            named_image: None,
            audio: None,
            ffmpeg: None,
            work_dir: work,
            fallback_color: [1, 2, 3],
            convert: plain,
        }
    }

    fn live() -> (watch::Sender<bool>, watch::Receiver<bool>) {
        watch::channel(false)
    }

    /// ffmpeg du système : les tests qui en ont besoin sont ignorés sans lui (il n'est pas
    /// installé partout, et le ffmpeg embarqué n'a pas d'encodeur d'image pour les fixtures).
    fn system_ffmpeg() -> Option<PathBuf> {
        let name = if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        };
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|d| d.join(name))
            .find(|p| p.is_file())
    }

    /// Fixture audio FLAC (avec ou sans image jointe) fabriquée par ffmpeg.
    fn make_flac(ffmpeg: &Path, dir: &Path, name: &str, cover: Option<&Path>) -> PathBuf {
        let out = dir.join(name);
        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=d=1"]);
        if let Some(c) = cover {
            cmd.arg("-i").arg(c);
            cmd.args(["-map", "0", "-map", "1", "-c:v", "copy"]);
            cmd.args(["-disposition:v", "attached_pic"]);
        }
        let status = cmd
            .args(["-c:a", "flac"])
            .arg(&out)
            .stdin(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "fixture {name}");
        out
    }

    #[test]
    fn arguments_ffmpeg_fixes() {
        let args = cover_args(Path::new("/a/b.mp3"), Path::new("/w/o.bin"));
        let s: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let at = |x: &str| s.iter().position(|a| a == x).unwrap();
        assert_eq!(s[at("-protocol_whitelist") + 1], "file");
        assert!(at("-protocol_whitelist") < at("-i"), "avant l'entrée");
        assert_eq!(s[at("-i") + 1], "/a/b.mp3");
        assert_eq!(s[at("-map") + 1], "0:v:0");
        assert_eq!(s[at("-c:v") + 1], "copy");
        assert_eq!(s[at("-frames:v") + 1], "1");
        assert!(s.iter().any(|a| a == "-fs"));
        assert_eq!(s.last().unwrap(), "/w/o.bin");
        // Une seule entrée, une seule sortie.
        assert_eq!(s.iter().filter(|a| *a == "-i").count(), 1);
    }

    #[tokio::test]
    async fn chaine_image_fournie_puis_titre_puis_unie() {
        let work = tempfile::tempdir().unwrap();
        let (_tx, mut rx) = live();

        // 1. Image du même nom.
        let named = work.path().join("histoire.png");
        std::fs::write(&named, png(30, 20, [200, 10, 10])).unwrap();
        let mut req = request(work.path(), "Titre");
        req.named_image = Some(&named);
        let got = complete_image(req, &mut rx).await.unwrap();
        assert_eq!(got.origin, ImageOrigin::Named);
        assert_eq!((got.artwork.width, got.artwork.height), (640, 480));

        // Image fournie illisible : image du titre.
        std::fs::write(&named, b"pas une image").unwrap();
        let mut req = request(work.path(), "Titre");
        req.named_image = Some(&named);
        assert_eq!(
            complete_image(req, &mut rx).await.unwrap().origin,
            ImageOrigin::TitleCard
        );

        // Absente, titre sans caractère affichable : image unie.
        let mut req = request(work.path(), "🎉");
        let missing = work.path().join("absente.png");
        req.named_image = Some(&missing);
        let got = complete_image(req, &mut rx).await.unwrap();
        assert_eq!(got.origin, ImageOrigin::Solid);
        let img = image::load_from_memory(&got.artwork.png).unwrap().to_rgb8();
        assert_eq!(img.get_pixel(5, 5).0, [1, 2, 3]);
    }

    #[tokio::test]
    async fn chaine_annulee() {
        let work = tempfile::tempdir().unwrap();
        let (tx, mut rx) = live();
        tx.send(true).unwrap();
        let err = complete_image(request(work.path(), "T"), &mut rx)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Cancelled), "{err:?}");
    }

    #[tokio::test]
    async fn pochette_chemin_refuse() {
        let work = tempfile::tempdir().unwrap();
        let (_tx, mut rx) = live();
        let fake = Verified::unchecked(work.path().join("jamais-lance"));
        for bad in [Path::new("relatif.mp3"), Path::new("-i"), work.path()] {
            let r = extract_cover(&fake, bad, work.path(), &mut rx).await;
            assert!(r.is_err(), "{bad:?}");
        }
    }

    #[tokio::test]
    async fn pochette_presente_absente_et_chaine() {
        let Some(ffmpeg) = system_ffmpeg() else {
            eprintln!("ffmpeg absent du PATH : test ignoré");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let cover = dir.path().join("c.png");
        std::fs::write(&cover, png(64, 48, [0, 160, 0])).unwrap();
        let with = make_flac(&ffmpeg, dir.path(), "avec.flac", Some(&cover));
        let without = make_flac(&ffmpeg, dir.path(), "sans.flac", None);
        let verified = Verified::unchecked(&ffmpeg);
        let (_tx, mut rx) = live();

        let bytes = extract_cover(&verified, &with, work.path(), &mut rx)
            .await
            .unwrap()
            .expect("pochette");
        assert_eq!(bytes, std::fs::read(&cover).unwrap());
        assert_eq!(
            extract_cover(&verified, &without, work.path(), &mut rx)
                .await
                .unwrap(),
            None,
            "absence de pochette : pas une erreur"
        );
        // Un fichier qui n'est pas de l'audio : pas de pochette non plus.
        let junk = dir.path().join("x.mp3");
        std::fs::write(&junk, b"ceci n'est pas de l'audio").unwrap();
        assert_eq!(
            extract_cover(&verified, &junk, work.path(), &mut rx)
                .await
                .unwrap(),
            None
        );
        // Dossier de travail propre, dossier source intact.
        assert_eq!(std::fs::read_dir(work.path()).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 4);

        // Chaîne : pochette, puis image du titre sans pochette.
        let mut req = request(work.path(), "Chapitre un");
        req.audio = Some(&with);
        req.ffmpeg = Some(&verified);
        let got = complete_image(req, &mut rx).await.unwrap();
        assert_eq!(got.origin, ImageOrigin::Cover);
        let img = image::load_from_memory(&got.artwork.png).unwrap().to_rgb8();
        assert_eq!(img.dimensions(), (640, 480));
        assert_eq!(img.get_pixel(320, 240).0, [0, 160, 0]);

        let mut req = request(work.path(), "Chapitre un");
        req.audio = Some(&without);
        req.ffmpeg = Some(&verified);
        assert_eq!(
            complete_image(req, &mut rx).await.unwrap().origin,
            ImageOrigin::TitleCard
        );
    }
}
