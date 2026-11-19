//! Fabrication d'un pack à partir de **fichiers locaux** : étape commune aux packs de podcasts
//! (`install::podcast`, après les téléchargements) et aux histoires faites d'un dossier
//! (`authoring::build`, note `docs/creation-dossier.md` § 11, lots 4 et 5).
//!
//! Le module ne connaît ni flux RSS ni épisode : un pack y est une liste d'éléments
//! ([`LocalItem`] : audio local, titre, image facultative, titre lu facultatif) rangés dans un
//! arbre de menus ([`MenuNode`]) qui les désigne par leur **rang** dans la liste. Les étapes
//! ([`PackBuild`]) écrivent dans le dossier de travail de la tâche :
//!
//! 1. titres lus ([`PackBuild::speak_titles`], 0 à [`SPEECH_PERCENT`] %, phase `speech`) ;
//! 2. MP3 conformes, avec le cache des MP3 convertis ([`PackBuild::convert_audio`], phase
//!    `audio` à 60 %) ;
//! 3. images 640 × 480 et couverture 512 × 512 ([`PackBuild::begin_images`] puis
//!    [`PackBuild::images`], phase `images` à 85 %) ;
//! 4. archive Telmi synthétique ([`PackBuild::write_archive`], phase `archive` à 90 %).
//!
//! [`PackBuild::build`] les enchaîne pour une source sans téléchargement. Les podcasts les
//! appellent une à une : leurs titres lus passent **avant** les téléchargements (voir
//! [`pack_speech`](super::pack_speech)) et leur couverture peut être téléchargée entre la
//! conversion audio et les images. La conversion `telmi2flam` et l'installation dans « Mes
//! histoires » suivent dans la file d'installation (`InstallService::produce_prepared`), à partir
//! de l'archive écrite ici.
//!
//! # Invariants
//!
//! - **Fichiers produits nommés par rang** (`ep-NNNN.mp3`, `ep-NNNN.png` par rang d'élément,
//!   `title-NNNN.mp3` par rang dans la liste des titres lus, `cover.png`, `pack.zip`) dans le
//!   dossier de travail : aucun nom de fichier source n'est réutilisé. Les fichiers source
//!   (précondition : **hors du dossier de travail**) ne sont jamais modifiés ; seuls les
//!   originaux téléchargés dans le dossier de travail (`*.audio`, `*.img`) sont supprimés avant
//!   l'archive.
//! - **Retraits** (P7) : un élément sans audio ([`ItemAudio::Missing`]), dont la conversion
//!   échoue ou dont le MP3 dépasse [`Limits::max_audio_bytes`] est retiré du pack, cité dans
//!   [`PackStatus::dropped`](super::podcast::PackStatus) et dans la description ; une catégorie
//!   vidée est retirée. Convertisseur indisponible ou erreur disque générale : arrêt du pack.
//! - **Cache des MP3** (#460) : au mieux, ses erreurs ne font jamais échouer le pack ; les
//!   éléments du pack ne sont jamais évincés.
//! - **Image illisible** (D13) : image de repli, l'élément reste.
//! - **Annulation** vérifiée entre les étapes et pendant la conversion audio et l'écriture de
//!   l'archive (`AppError::Cancelled`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use super::pack_speech::{self, SpokenTitles, TitleKey};
use super::podcast::{BoxFuture, PackBackend, PackStatus};
use super::{lock, InstallService, TaskState};
use crate::catalog::blocking;
use crate::ffmpeg::Verified;
use crate::podcast::archive::{
    self, ArchiveOptions, Limits, MenuEntry, MenuMode, Pack, PackEpisode, PackMetadata,
};
use crate::podcast::artwork::{self, Artwork, ArtworkError, ArtworkKind};
use crate::podcast::fetch::{self, Workdir};
use crate::podcast::mp3cache::{self, CacheAccess, EntryKey};
use crate::podcast::pack::ResolvedMetadata;
use crate::podcast::speech::{self, Synthesizer, Wav};
use crate::story::StoryEntry;
use crate::{AppError, AppResult};

/// Part de la préparation réservée aux titres lus (#487) : 0 à 10 % ; les téléchargements d'un
/// pack de podcasts suivent de 10 à 60 %.
pub const SPEECH_PERCENT: f64 = 10.0;
/// Archive Telmi, dans le dossier de travail.
pub const ARCHIVE_NAME: &str = "pack.zip";

/// Accès externes de la fabrication (conversion audio et images, voix). Implémenté pour tout
/// [`PackBackend`] (podcasts) ; une autre source peut l'implémenter seule.
pub trait BuildBackend: Send + Sync {
    /// Conversion audio (#214) de `(source, MP3 produit)` : taille de chaque MP3 produit, ou son
    /// erreur (P7).
    fn convert_audio<'a>(
        &'a self,
        items: Vec<(PathBuf, PathBuf)>,
        cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<Vec<AppResult<u64>>>>;
    /// Conversion d'une image (#215), bloquante. Par défaut dans un processus enfant (#263).
    fn convert_artwork(&self, bytes: &[u8], kind: ArtworkKind) -> Result<Artwork, ArtworkError> {
        artwork::convert_isolated(bytes, kind, None)
    }
    /// Moteur de synthèse des titres lus (#462) : voix Windows par défaut.
    fn speech(&self) -> Arc<dyn Synthesizer> {
        speech::system()
    }
    /// Délai de synthèse d'un titre.
    fn synth_timeout(&self) -> Duration {
        speech::SYNTH_TIMEOUT
    }
    /// Convertisseur des titres lus, préparé une seule fois pour l'étape (#487).
    fn title_encoder<'a>(
        &'a self,
        cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<Verified>> {
        Box::pin(live_title_encoder(cancel))
    }
    /// Conversion d'un titre synthétisé en MP3 `dest` ; taille du MP3.
    fn title_mp3<'a>(
        &'a self,
        encoder: &'a Verified,
        wav: &'a Wav,
        dest: &'a Path,
        cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<u64>> {
        Box::pin(live_title_mp3(encoder, wav, dest, cancel))
    }
    /// ffmpeg vérifié qui extrait la pochette d'un fichier audio (compléments d'image d'une
    /// histoire faite d'un dossier, `authoring::images`) : par défaut, celui des titres lus.
    fn cover_extractor<'a>(
        &'a self,
        cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<Verified>> {
        self.title_encoder(cancel)
    }
    /// Décodage des images des compléments (pochette extraite), bloquant : par défaut dans un
    /// processus enfant (#263), comme [`Self::convert_artwork`].
    fn image_decoder(&self) -> crate::authoring::images::Convert {
        crate::authoring::images::convert_in_child
    }
}

/// ffmpeg embarqué localisé, empreinte vérifiée (défaut de `title_encoder`).
pub(super) async fn live_title_encoder(cancel: &watch::Receiver<bool>) -> AppResult<Verified> {
    let ff = crate::ffmpeg::Ffmpeg::locate(&crate::settings::Environment::current())?;
    let mut cancel = cancel.clone();
    ff.verify(crate::ffmpeg::Tool::Ffmpeg, &mut cancel).await
}

/// Titre synthétisé converti par le ffmpeg vérifié (défaut de `title_mp3`).
pub(super) async fn live_title_mp3(
    encoder: &Verified,
    wav: &Wav,
    dest: &Path,
    cancel: &watch::Receiver<bool>,
) -> AppResult<u64> {
    let mut cancel = cancel.clone();
    speech::wav_to_mp3_verified(encoder, wav, dest, &mut cancel).await
}

/// Un accès de pack de podcasts sert la fabrication telle quelle.
impl<T: PackBackend + ?Sized> BuildBackend for T {
    fn convert_audio<'a>(
        &'a self,
        items: Vec<(PathBuf, PathBuf)>,
        cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<Vec<AppResult<u64>>>> {
        PackBackend::convert_audio(self, items, cancel)
    }
    fn convert_artwork(&self, bytes: &[u8], kind: ArtworkKind) -> Result<Artwork, ArtworkError> {
        PackBackend::convert_artwork(self, bytes, kind)
    }
    fn speech(&self) -> Arc<dyn Synthesizer> {
        PackBackend::speech(self)
    }
    fn synth_timeout(&self) -> Duration {
        PackBackend::synth_timeout(self)
    }
    fn title_encoder<'a>(
        &'a self,
        cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<Verified>> {
        PackBackend::title_encoder(self, cancel)
    }
    fn title_mp3<'a>(
        &'a self,
        encoder: &'a Verified,
        wav: &'a Wav,
        dest: &'a Path,
        cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<u64>> {
        PackBackend::title_mp3(self, encoder, wav, dest, cancel)
    }
}

/// Option d'un menu : élément (rang dans [`PackSpec::items`]) ou catégorie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuNode {
    Item(usize),
    Category {
        name: String,
        entries: Vec<MenuNode>,
        /// Image choisie (N5 c) : celle de l'élément de ce rang, s'il est dans la catégorie et
        /// garde une image ; sinon image automatique et avertissement.
        image: Option<usize>,
        /// Mode propre (`MenuEntry::Submenu`, histoires sur dossier) ; `None` : la catégorie
        /// hérite du mode de son menu (`MenuEntry::Category`, podcasts).
        mode: Option<MenuMode>,
        /// Image propre du sous-menu (`_image.*` d'un dossier), de format quelconque :
        /// convertie par [`PackBuild::pictures`], prioritaire sur `image`. Ne sert qu'avec un
        /// `mode` (podcasts : toujours `None`).
        picture: Option<PathBuf>,
    },
}

/// Source du pack : choisit les mots des messages montrés à l'utilisateur (un pack de
/// podcasts parle d'**épisodes**, une histoire sur dossier d'**histoires**), sans changer ceux
/// des podcasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BuildSource {
    #[default]
    Podcast,
    Folder,
}

impl BuildSource {
    /// ffmpeg embarqué indisponible.
    fn converter_missing(self, why: &str) -> String {
        match self {
            Self::Podcast => format!(
                "ffmpeg embarqué indisponible, nécessaire pour convertir les épisodes du pack \
                 (réinstallez Flam Launcher) : {why}"
            ),
            Self::Folder => format!(
                "ffmpeg embarqué indisponible, nécessaire pour convertir l'audio de l'histoire \
                 (réinstallez Flam Launcher) : {why}"
            ),
        }
    }

    /// Aucun élément gardé après la conversion audio.
    fn nothing_converted(self, titles: &str) -> String {
        match self {
            Self::Podcast => format!(
                "aucun épisode utilisable (téléchargement ou conversion en échec) : {titles}"
            ),
            Self::Folder => format!("aucune histoire utilisable (conversion en échec) : {titles}"),
        }
    }

    /// Arborescence vide après les retraits.
    fn nothing_left(self) -> &'static str {
        match self {
            Self::Podcast => "aucun épisode utilisable dans le pack",
            Self::Folder => "aucune histoire utilisable",
        }
    }

    /// En-tête de la liste des éléments retirés, dans la description.
    fn failures_head(self) -> &'static str {
        match self {
            Self::Podcast => archive::FAILURES_HEAD,
            Self::Folder => "Histoires non incluses (échec de la conversion) :",
        }
    }
}

impl MenuNode {
    /// Vrai si l'élément `rank` figure dans `menu` (à toute profondeur).
    pub fn contains_item(menu: &[MenuNode], rank: usize) -> bool {
        menu.iter().any(|node| match node {
            MenuNode::Item(n) => *n == rank,
            MenuNode::Category { entries, .. } => Self::contains_item(entries, rank),
        })
    }
}

/// Audio d'un élément.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemAudio {
    /// Fichier local à convertir en MP3 conforme.
    Source(PathBuf),
    /// MP3 déjà conforme (cache des MP3) et sa taille.
    Ready { mp3: PathBuf, bytes: u64 },
    /// Audio indisponible (téléchargement en échec) : élément retiré du pack.
    Missing,
}

/// Élément du pack (histoire, épisode, chapitre). Ses fichiers source sont lus, jamais modifiés,
/// et doivent être **hors du dossier de travail** : `*.audio` et `*.img` y sont supprimés avant
/// l'archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalItem {
    /// Titre affiché (et lu, si les titres lus sont demandés).
    pub title: String,
    pub audio: ItemAudio,
    /// Image source locale, de format quelconque (convertie en 640 × 480) ; illisible ou
    /// absente : image de repli (D13).
    pub image: Option<PathBuf>,
    /// MP3 conforme du titre lu fourni par la source. La synthèse ne le remplace jamais : un
    /// élément qui en a un n'est pas proposé à la voix ([`PackBuild::speak_titles`]).
    pub title_audio: Option<PathBuf>,
    /// Clé du cache des MP3 convertis ; `None` : l'élément n'y passe pas.
    pub cache_key: Option<EntryKey>,
}

/// Pack à fabriquer.
#[derive(Debug, Clone)]
pub struct PackSpec {
    /// Uuid du pack, forme canonique.
    pub uuid: String,
    pub metadata: ResolvedMetadata,
    /// Éléments, par rang.
    pub items: Vec<LocalItem>,
    /// Menu racine.
    pub menu: Vec<MenuNode>,
    /// Mode du menu racine et bornes de l'archive (podcasts : [`ArchiveOptions::default`]).
    pub options: ArchiveOptions,
}

/// Titres lus demandés (#462) : voix choisie (`None` : voix française par défaut).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TitlesAudio {
    pub voice: Option<String>,
}

/// Élément gardé après la conversion audio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    pub rank: usize,
    pub mp3: PathBuf,
    /// Image source (pas encore convertie).
    pub image: Option<PathBuf>,
}

/// Résultat de la conversion audio.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Converted {
    pub kept: Vec<Kept>,
    /// Titres des éléments retirés, par rang.
    pub dropped_titles: Vec<String>,
}

/// Images converties : PNG par rang, couverture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Images {
    pub items: HashMap<usize, PathBuf>,
    pub cover: Option<PathBuf>,
}

/// Panne du convertisseur audio : fatale pour le pack entier, jamais comptée comme un échec
/// d'élément (message des podcasts).
pub(super) fn converter_unavailable(err: AppError) -> AppError {
    converter_unavailable_for(BuildSource::Podcast, err)
}

/// [`converter_unavailable`] avec les mots de `source`.
pub(crate) fn converter_unavailable_for(source: BuildSource, err: AppError) -> AppError {
    match err {
        AppError::ConverterUnavailable(why) => {
            AppError::ConverterUnavailable(source.converter_missing(&why))
        }
        other => other,
    }
}

/// Erreur de conversion qui arrête tout le pack au lieu de retirer l'élément : convertisseur
/// indisponible, ou erreur disque générale (hors délai d'un élément, `TimedOut`). Les erreurs
/// disque propres à un élément arrivent ici en `Conversion` (`io_failed`) et le retirent.
pub(crate) fn is_fatal_conversion(err: &AppError) -> bool {
    match err {
        AppError::ConverterUnavailable(_) => true,
        AppError::Io(e) => e.kind() != std::io::ErrorKind::TimedOut,
        _ => false,
    }
}

/// Motif journalisé d'un élément retiré : genre de l'erreur et, pour une conversion, son code
/// stable. Ni URL ni titre.
pub(super) fn drop_reason(err: &AppError) -> String {
    match err {
        AppError::Conversion { code, .. } => format!("{} ({code})", err.kind()),
        _ => err.kind().to_string(),
    }
}

impl InstallService {
    /// Avancement de la préparation de la tâche `id`.
    pub(crate) fn emit_phase(&self, id: &str, percent: f64, phase: &str) {
        self.emit(super::InstallProgress {
            task_id: id.to_string(),
            state: TaskState::Preparing,
            percent: Some(percent),
            received: None,
            total: None,
            phase: Some(phase.to_string()),
        });
    }

    /// Nombre d'éléments et titres retirés de la tâche `id` (avertissements gardés).
    pub(crate) fn set_pack_status(&self, id: &str, episodes: usize, dropped: Vec<String>) {
        let mut tasks = lock(&self.0.tasks);
        if let Some(slot) = tasks.iter_mut().find(|s| s.task.id == id) {
            let warnings = slot
                .task
                .pack
                .take()
                .map(|p| p.warnings)
                .unwrap_or_default();
            slot.task.pack = Some(PackStatus {
                episodes,
                dropped,
                warnings,
            });
        }
    }

    fn set_pack_warnings(&self, id: &str, warnings: Vec<String>) {
        let mut tasks = lock(&self.0.tasks);
        if let Some(slot) = tasks.iter_mut().find(|s| s.task.id == id) {
            slot.task
                .pack
                .get_or_insert_with(PackStatus::default)
                .warnings = warnings;
        }
    }
}

/// Préparation terminée : dossier de travail (à supprimer par l'appelant), archive écrite,
/// entrée à publier.
pub(super) struct Prepared {
    pub workdir: Workdir,
    pub archive: PathBuf,
    pub entry: StoryEntry,
}

/// Étapes de fabrication d'un pack de la tâche `id`, dans `workdir`.
pub(crate) struct PackBuild<'a, B: ?Sized> {
    pub service: &'a InstallService,
    pub id: &'a str,
    pub workdir: &'a Path,
    pub backend: &'a Arc<B>,
    pub cancel: &'a watch::Receiver<bool>,
    /// Mots des messages ([`BuildSource`]).
    pub source: BuildSource,
}

impl<B: BuildBackend + ?Sized + 'static> PackBuild<'_, B> {
    fn cancelled(&self) -> AppResult<()> {
        if *self.cancel.borrow() {
            return Err(AppError::Cancelled);
        }
        Ok(())
    }

    /// Toutes les étapes, pour des éléments déjà locaux : titres lus si `titles` est donné,
    /// audio, images (couverture tirée de `cover`), archive. Rend le chemin de l'archive.
    // Enchaînement de référence des étapes ; essayé dans `pack_build_tests.rs` (la source
    // « dossier » les appelle une à une : `authoring::build`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub async fn build(
        &self,
        spec: &PackSpec,
        cover: Option<PathBuf>,
        titles: Option<&TitlesAudio>,
        cache: Option<&CacheAccess>,
    ) -> AppResult<PathBuf> {
        self.service
            .set_pack_status(self.id, spec.items.len(), Vec::new());
        let (spoken, warnings) = match titles {
            Some(titles) => {
                self.speak_titles(
                    &spec.menu,
                    &spec.items,
                    &spec.metadata.title,
                    titles.voice.as_deref(),
                    &HashSet::new(),
                )
                .await?
            }
            None => (SpokenTitles::default(), Vec::new()),
        };
        // Annulation déjà demandée : aucune conversion lancée.
        self.cancelled()?;
        let converted = self
            .convert_audio(
                &spec.items,
                &spec.options.limits,
                cache,
                Vec::new(),
                super::now_secs(),
            )
            .await?;
        self.begin_images();
        let mut spec = spec.clone();
        self.pictures(&mut spec.menu).await?;
        let images = self.images(&converted, cover).await?;
        self.write_archive(&spec, converted, images, &spoken, warnings)
            .await
    }

    /// Titres lus (#462) : voix choisie (repli sur la voix par défaut avec un avertissement),
    /// puis synthèse et conversion de chaque titre (titre du pack, puis options dans l'ordre de
    /// l'archive). Ne sont **pas** proposés à la voix : les emplacements de `provided` (titres
    /// lus fournis par la source pour le pack ou une catégorie) et les éléments qui ont déjà un
    /// titre lu ([`LocalItem::title_audio`]) : sinon un échec de synthèse avertirait « option
    /// muette » alors que le titre fourni reste lu. Rend les MP3 produits et les avertissements
    /// de voix ; voir [`pack_speech::speak`] pour les arrêts.
    pub async fn speak_titles(
        &self,
        menu: &[MenuNode],
        items: &[LocalItem],
        pack_title: &str,
        voice: Option<&str>,
        provided: &HashSet<TitleKey>,
    ) -> AppResult<(SpokenTitles, Vec<String>)> {
        self.service.emit_phase(self.id, 0.0, "speech");
        let synth = self.backend.speech();
        let (voice, fallback) = {
            let (synth, requested) = (synth.clone(), voice.map(str::to_string));
            blocking(move || pack_speech::choose_voice(&synth, requested.as_deref())).await??
        };
        let mut titles = pack_speech::menu_titles(menu, pack_title, &|n| {
            items
                .get(n)
                .filter(|item| item.title_audio.is_none())
                .map(|item| item.title.clone())
        });
        titles.retain(|t| !provided.contains(&t.key));
        // Avancement par titre (0 à SPEECH_PERCENT, #487) ; durée réelle journalisée (nombres
        // seulement, jamais de texte ni de voix).
        let started = std::time::Instant::now();
        let progress = |done: usize, total: usize| {
            if total > 0 {
                let part = done as f64 / total as f64;
                self.service
                    .emit_phase(self.id, part * SPEECH_PERCENT, "speech");
            }
        };
        let spoken = pack_speech::speak(
            &**self.backend,
            &synth,
            &voice,
            &titles,
            self.workdir,
            self.cancel,
            &progress,
        )
        .await?;
        tracing::info!(
            titles = spoken.audio.len(),
            failed = spoken.failed.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "titres lus : étape terminée"
        );
        Ok((spoken, fallback.into_iter().collect()))
    }

    /// MP3 conformes (phase `audio`, 60 %) : conversion des sources dans l'ordre des rangs,
    /// MP3 déjà conformes repris tels quels, retraits (P7). Les MP3 convertis rejoignent le
    /// cache avec `to_cache` (entrées déjà prêtes à y remettre), puis le cache est évincé (jamais
    /// les éléments du pack). Met à jour [`PackStatus`] (éléments, titres retirés).
    pub async fn convert_audio(
        &self,
        items: &[LocalItem],
        limits: &Limits,
        cache: Option<&CacheAccess>,
        mut to_cache: Vec<mp3cache::Store>,
        now: u64,
    ) -> AppResult<Converted> {
        self.service.emit_phase(self.id, 60.0, "audio");
        let mut dropped: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.audio == ItemAudio::Missing)
            .map(|(n, _)| n)
            .collect();
        let mut conversions = Vec::new();
        for (n, item) in items.iter().enumerate() {
            if let ItemAudio::Source(src) = &item.audio {
                let mp3 = self.workdir.join(format!("ep-{n:04}.mp3"));
                conversions.push((src.clone(), mp3));
            }
        }
        let results = if conversions.is_empty() {
            Vec::new()
        } else {
            self.backend
                .convert_audio(conversions.clone(), self.cancel)
                .await?
        };
        self.cancelled()?;
        let mut results = conversions.into_iter().zip(results);
        let mut kept: Vec<Kept> = Vec::new();
        let mut audio_total = 0u64;
        for (n, item) in items.iter().enumerate() {
            match &item.audio {
                ItemAudio::Missing => {}
                // MP3 déjà conforme : le cache ne sert jamais une entrée au-delà de
                // `MAX_AUDIO_BYTES` (`CacheAccess::lookup` la supprime, #466) ; filet pour une
                // autre source.
                ItemAudio::Ready { mp3, bytes } => {
                    if *bytes <= limits.max_audio_bytes {
                        audio_total += bytes;
                        kept.push(Kept {
                            rank: n,
                            mp3: mp3.clone(),
                            image: item.image.clone(),
                        });
                    } else {
                        tracing::warn!(item = n, "élément retiré du pack : MP3 trop volumineux");
                        dropped.push(n);
                    }
                }
                ItemAudio::Source(_) => {
                    // Un résultat par conversion demandée, dans l'ordre des rangs.
                    let Some(((_, mp3), result)) = results.next() else {
                        continue;
                    };
                    match result {
                        Ok(bytes) if bytes <= limits.max_audio_bytes => {
                            audio_total += bytes;
                            if let Some(key) = &item.cache_key {
                                to_cache.push(mp3cache::Store {
                                    key: key.clone(),
                                    mp3: mp3.clone(),
                                    image: item.image.clone(),
                                    image_only: false,
                                });
                            }
                            kept.push(Kept {
                                rank: n,
                                mp3,
                                image: item.image.clone(),
                            });
                        }
                        // Filet : `audio::convert` refuse déjà toute sortie au-delà de la borne
                        // (`audio_too_large`), branche inatteignable avec le convertisseur réel.
                        Ok(_) => {
                            tracing::warn!(
                                item = n,
                                "élément retiré du pack : MP3 trop volumineux"
                            );
                            dropped.push(n);
                        }
                        Err(AppError::Cancelled) => return Err(AppError::Cancelled),
                        Err(err) if is_fatal_conversion(&err) => {
                            return Err(converter_unavailable_for(self.source, err))
                        }
                        Err(err) => {
                            tracing::warn!(
                                item = n,
                                "élément retiré du pack : {}",
                                drop_reason(&err)
                            );
                            dropped.push(n);
                        }
                    }
                }
            }
        }
        // Mise en cache des MP3 convertis, avant que les images d'origine ne soient supprimées
        // du dossier de travail ; puis éviction (jamais les éléments de ce pack).
        if let Some(cache) = cache {
            if !to_cache.is_empty() {
                cache.store(to_cache, now).await;
            }
            let protect: HashSet<(String, String)> = items
                .iter()
                .filter_map(|item| item.cache_key.as_ref())
                .map(|k| (k.source_id.clone(), k.episode_id.clone()))
                .collect();
            cache.evict(protect).await;
        }
        self.cancelled()?;
        if kept.is_empty() {
            let titles: Vec<String> = dropped
                .iter()
                .take(3)
                .filter_map(|&n| items.get(n).map(|item| archive::label(&item.title, "?")))
                .collect();
            let more = if dropped.len() > 3 { ", …" } else { "" };
            return Err(AppError::DownloadInvalid(
                self.source
                    .nothing_converted(&format!("{}{more}", titles.join(", "))),
            ));
        }
        if audio_total > limits.max_total_audio_bytes {
            return Err(AppError::DownloadInvalid(format!(
                "audio converti trop volumineux : {} Mo",
                audio_total.div_ceil(1_000_000)
            )));
        }
        dropped.sort_unstable();
        dropped.dedup();
        let dropped_titles: Vec<String> = dropped
            .iter()
            .filter_map(|&n| items.get(n).map(|item| item.title.clone()))
            .collect();
        self.service
            .set_pack_status(self.id, items.len(), dropped_titles.clone());
        Ok(Converted {
            kept,
            dropped_titles,
        })
    }

    /// Début de la phase `images` (85 %). Séparé de [`Self::images`] : un pack de podcasts
    /// télécharge sa couverture entre les deux.
    pub fn begin_images(&self) {
        self.service.emit_phase(self.id, 85.0, "images");
    }

    /// Images des éléments gardés (640 × 480) et couverture `cover` (512 × 512) en PNG ; une
    /// image illisible ne retire pas l'élément (D13).
    pub async fn images(&self, converted: &Converted, cover: Option<PathBuf>) -> AppResult<Images> {
        self.cancelled()?;
        let dir = self.workdir.to_path_buf();
        let jobs: Vec<(usize, Option<PathBuf>)> = converted
            .kept
            .iter()
            .map(|k| (k.rank, k.image.clone()))
            .collect();
        let converter = self.backend.clone();
        let (items, cover) = blocking(move || {
            convert_artwork(&dir, &jobs, cover, &|bytes, kind| {
                converter.convert_artwork(bytes, kind)
            })
        })
        .await??;
        Ok(Images { items, cover })
    }

    /// Images propres des sous-menus ([`MenuNode::Category::picture`]) converties en PNG
    /// 640 × 480 dans le dossier de travail (`menu-NNNN.png`, par ordre du parcours) ; chaque
    /// `picture` désigne ensuite son PNG. Illisible : `None` (image automatique, D13).
    pub async fn pictures(&self, menu: &mut [MenuNode]) -> AppResult<()> {
        fn collect(menu: &[MenuNode], out: &mut Vec<PathBuf>) {
            for node in menu {
                if let MenuNode::Category {
                    entries, picture, ..
                } = node
                {
                    if let Some(src) = picture {
                        out.push(src.clone());
                    }
                    collect(entries, out);
                }
            }
        }
        fn assign(menu: &mut [MenuNode], pngs: &mut std::vec::IntoIter<Option<PathBuf>>) {
            for node in menu {
                if let MenuNode::Category {
                    entries, picture, ..
                } = node
                {
                    if picture.is_some() {
                        *picture = pngs.next().flatten();
                    }
                    assign(entries, pngs);
                }
            }
        }
        self.cancelled()?;
        let mut sources = Vec::new();
        collect(menu, &mut sources);
        if sources.is_empty() {
            return Ok(());
        }
        let dir = self.workdir.to_path_buf();
        let converter = self.backend.clone();
        let flag = self.cancel.clone();
        let pngs = blocking(move || {
            let mut out = Vec::with_capacity(sources.len());
            for (n, src) in sources.iter().enumerate() {
                if *flag.borrow() {
                    return Err(AppError::Cancelled);
                }
                let dest = dir.join(format!("menu-{n:04}.png"));
                let png =
                    read_bounded(src, fetch::MAX_IMAGE_BYTES).and_then(|bytes| {
                        match converter.convert_artwork(&bytes, ArtworkKind::Episode) {
                            Ok(art) => Some(art.png),
                            Err(err) => {
                                tracing::warn!("image de menu ignorée : {err}");
                                None
                            }
                        }
                    });
                match png {
                    Some(png) => {
                        std::fs::write(&dest, png)?;
                        out.push(Some(dest));
                    }
                    None => out.push(None),
                }
            }
            Ok::<_, AppError>(out)
        })
        .await??;
        assign(menu, &mut pngs.into_iter());
        Ok(())
    }

    /// Archive Telmi (phase `archive`, 90 %) : arborescence (éléments retirés sautés,
    /// catégories vidées retirées), titres lus, avertissements de la tâche, description
    /// (par défaut : liste des titres ; éléments retirés ajoutés). Rend le chemin de l'archive.
    pub async fn write_archive(
        &self,
        spec: &PackSpec,
        converted: Converted,
        images: Images,
        spoken: &SpokenTitles,
        mut warnings: Vec<String>,
    ) -> AppResult<PathBuf> {
        self.service.emit_phase(self.id, 90.0, "archive");
        let Images { items: pngs, cover } = images;
        let mut episodes: HashMap<usize, PackEpisode> = HashMap::new();
        for kept in converted.kept {
            let Some(item) = spec.items.get(kept.rank) else {
                continue;
            };
            episodes.insert(
                kept.rank,
                PackEpisode {
                    title: item.title.clone(),
                    audio: kept.mp3,
                    image: pngs.get(&kept.rank).cloned(),
                    title_audio: item.title_audio.clone(),
                },
            );
        }
        let mut used: HashSet<TitleKey> = HashSet::new();
        let mut menu = build_menu(
            &spec.menu,
            &mut episodes,
            spoken,
            &mut Vec::new(),
            &mut used,
            &mut warnings,
        );
        mute_shifted_categories(&spec.menu, &used, spoken, &mut menu);
        if menu.is_empty() {
            return Err(AppError::DownloadInvalid(self.source.nothing_left().into()));
        }
        // Avertissements des seuls titres muets qui restent dans le pack (un élément retiré
        // n'en a plus besoin).
        let title_audio = spoken.audio.get(&TitleKey::Pack).cloned();
        warnings.extend(
            spoken
                .failed
                .iter()
                .filter(|(key, _)| *key == TitleKey::Pack || used.contains(key))
                .map(|(key, text)| pack_speech::failure_warning(key, text)),
        );
        if !warnings.is_empty() {
            self.service.set_pack_warnings(self.id, warnings);
        }
        let metadata = &spec.metadata;
        let description = metadata
            .description
            .clone()
            .unwrap_or_else(|| archive::default_description(&menu));
        let pack = Pack {
            uuid: uuid::Uuid::parse_str(&spec.uuid)
                .map_err(|_| AppError::Validation("uuid de pack invalide".into()))?,
            metadata: PackMetadata {
                title: metadata.title.clone(),
                subtitle: metadata.subtitle.clone(),
                age: metadata.age,
                description: fit_description(archive::with_failures_under(
                    &description,
                    &converted.dropped_titles,
                    self.source.failures_head(),
                )),
            },
            cover,
            menu,
            title_audio,
        };
        let archive_path = self.workdir.join(ARCHIVE_NAME);
        let dest = archive_path.clone();
        let flag = self.cancel.clone();
        let options = spec.options;
        blocking(move || archive::write_archive_with(&pack, &options, &dest, &|| *flag.borrow()))
            .await?
            .map_err(|err| match err {
                archive::ArchiveError::Cancelled => AppError::Cancelled,
                archive::ArchiveError::Io(e) => AppError::Io(e),
                other => AppError::Conversion {
                    code: "pack_archive".into(),
                    message: other.to_string(),
                },
            })?;
        Ok(archive_path)
    }
}

/// Décodeur d'image ([`BuildBackend::convert_artwork`]).
pub(super) type ArtworkDecoder<'a> =
    dyn Fn(&[u8], ArtworkKind) -> Result<Artwork, ArtworkError> + 'a;

/// Convertit les images (640 × 480) et la couverture (512 × 512) en PNG dans `dir` ; supprime
/// les originaux téléchargés de `dir` (`*.audio`, `*.img`). Une image illisible donne l'image de
/// repli (D13). Bloquant.
pub(super) fn convert_artwork(
    dir: &Path,
    jobs: &[(usize, Option<PathBuf>)],
    cover: Option<PathBuf>,
    decode: &ArtworkDecoder<'_>,
) -> AppResult<(HashMap<usize, PathBuf>, Option<PathBuf>)> {
    let convert = |src: &Path, kind: ArtworkKind, dest: PathBuf| -> AppResult<Option<PathBuf>> {
        let bytes = match read_bounded(src, fetch::MAX_IMAGE_BYTES) {
            Some(b) => b,
            None => return Ok(None),
        };
        match decode(&bytes, kind) {
            Ok(art) => {
                std::fs::write(&dest, &art.png)?;
                Ok(Some(dest))
            }
            Err(err) => {
                tracing::warn!("pochette ignorée : {err}");
                Ok(None)
            }
        }
    };
    let cover_png = match &cover {
        Some(src) => convert(src, ArtworkKind::Cover, dir.join("cover.png"))?,
        None => None,
    };
    let mut pngs = HashMap::new();
    for (n, image) in jobs {
        if let Some(src) = image {
            if let Some(png) = convert(
                src,
                ArtworkKind::Episode,
                dir.join(format!("ep-{n:04}.png")),
            )? {
                pngs.insert(*n, png);
            }
        }
    }
    // Originaux inutiles pour l'archive : place libérée avant son écriture.
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".audio") || name.ends_with(".img") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Ok((pngs, cover_png))
}

/// Description bornée à [`archive::MAX_DESCRIPTION_BYTES`] : au-delà (centaines d'histoires
/// aux titres longs), seules les premières lignes entières sont gardées, suivies de `…`. En deçà,
/// inchangée (podcasts : jamais atteint).
fn fit_description(description: String) -> String {
    let max = archive::MAX_DESCRIPTION_BYTES;
    if description.len() <= max {
        return description;
    }
    let mut out = String::new();
    for line in description.lines() {
        // Place pour la ligne, son saut et la marque finale.
        if out.len() + line.len() + "\n…".len() + 1 > max {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.push('…');
    out
}

/// Lit au plus `max` octets (`None` si absent, illisible ou plus gros).
fn read_bounded(path: &Path, max: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(max + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= max).then_some(bytes)
}

/// Arborescence de l'archive : éléments retirés sautés, catégories vidées retirées (P7) ;
/// titres lus (#462) repris de `spoken` par position dans `menu` (`path`), positions gardées
/// notées dans `used`. Image choisie d'une catégorie (N5 c) : le PNG de l'élément désigné, pris
/// dans `episodes` avant leur retrait ; élément hors de la catégorie, retiré du pack ou sans
/// image : image automatique (N5 a) et avertissement dans `warnings`.
pub(super) fn build_menu(
    menu: &[MenuNode],
    episodes: &mut HashMap<usize, PackEpisode>,
    spoken: &SpokenTitles,
    path: &mut Vec<usize>,
    used: &mut HashSet<TitleKey>,
    warnings: &mut Vec<String>,
) -> Vec<MenuEntry> {
    let mut out = Vec::new();
    for (i, node) in menu.iter().enumerate() {
        path.push(i);
        let key = TitleKey::Node(path.clone());
        match node {
            MenuNode::Item(n) => {
                if let Some(mut ep) = episodes.remove(n) {
                    if let Some(audio) = spoken.audio.get(&key) {
                        ep.title_audio = Some(audio.clone());
                    }
                    out.push(MenuEntry::Episode(ep));
                    used.insert(key);
                }
            }
            MenuNode::Category {
                name,
                entries,
                image,
                mode,
                picture,
            } => {
                // Avant le retrait des éléments de la catégorie.
                let chosen: Option<Option<PathBuf>> = image.map(|n| {
                    MenuNode::contains_item(entries, n)
                        .then(|| episodes.get(&n))
                        .flatten()
                        .and_then(|ep| ep.image.clone())
                });
                let entries = build_menu(entries, episodes, spoken, path, used, warnings);
                if !entries.is_empty() {
                    let image = match chosen {
                        Some(Some(png)) => Some(png),
                        Some(None) => {
                            warnings.push(format!(
                                "Image de la catégorie « {} » : l'épisode choisi n'est plus \
                                 disponible, image automatique utilisée.",
                                archive::label(name, "sans nom")
                            ));
                            None
                        }
                        None => None,
                    };
                    let (name, audio) = (name.trim().to_string(), spoken.audio.get(&key).cloned());
                    out.push(match mode {
                        None => MenuEntry::Category {
                            name,
                            entries,
                            audio,
                            image,
                        },
                        Some(mode) => MenuEntry::Submenu {
                            name,
                            entries,
                            audio,
                            image,
                            picture: picture.clone(),
                            mode: *mode,
                        },
                    });
                    used.insert(key);
                }
            }
        }
        path.pop();
    }
    out
}

/// Titres lus « Catégorie k » (nom sans rien d'affichable, #487) : `k` est compté sur `tree`,
/// avant les retraits. Si une catégorie vidée plus haut (P7) décale la numérotation de
/// l'archive, le titre lu ne dit plus le libellé affiché : l'option reste alors muette.
fn mute_shifted_categories(
    tree: &[MenuNode],
    used: &HashSet<TitleKey>,
    spoken: &SpokenTitles,
    menu: &mut [MenuEntry],
) {
    if spoken.numbered.is_empty() {
        return;
    }
    // Catégories gardées, dans l'ordre de l'archive (une catégorie avant ses options).
    fn kept(
        menu: &[MenuNode],
        used: &HashSet<TitleKey>,
        path: &mut Vec<usize>,
        out: &mut Vec<TitleKey>,
    ) {
        for (i, node) in menu.iter().enumerate() {
            path.push(i);
            if let MenuNode::Category { entries, .. } = node {
                let key = TitleKey::Node(path.clone());
                if used.contains(&key) {
                    out.push(key);
                }
                kept(entries, used, path, out);
            }
            path.pop();
        }
    }
    fn audios<'a>(menu: &'a mut [MenuEntry], out: &mut Vec<&'a mut Option<PathBuf>>) {
        for entry in menu {
            if let MenuEntry::Category { entries, audio, .. }
            | MenuEntry::Submenu { entries, audio, .. } = entry
            {
                out.push(audio);
                audios(entries, out);
            }
        }
    }
    let mut keys = Vec::new();
    kept(tree, used, &mut Vec::new(), &mut keys);
    let mut slots = Vec::new();
    audios(menu, &mut slots);
    debug_assert_eq!(keys.len(), slots.len());
    for (rank, (key, audio)) in keys.iter().zip(slots).enumerate() {
        if spoken.numbered.get(key).is_some_and(|&k| k != rank + 1) {
            *audio = None;
        }
    }
}

#[cfg(test)]
#[path = "pack_build_tests.rs"]
mod tests;
