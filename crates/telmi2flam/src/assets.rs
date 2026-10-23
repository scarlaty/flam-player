//! Assets d'une histoire (`conv_image`, `conv_audio`, `add_mp3map`, `alloc` de `_convert` dans
//! `tools/telmi2flam/telmi2flam.py`) : noms de sortie, caches, conversions et avertissements.
//!
//! Invariants :
//! - noms de sortie de la forme `img/<nom>.lif` ou `sounds/<nom>.mp3[map]`, `<nom>` dans
//!   `[A-Za-z0-9_-]+` : aucun séparateur ni `..`, quel que soit le nom dans l'archive ;
//! - uniques sans tenir compte de la casse (FAT, Windows), noms réservés compris
//!   (`img/title.lif`, `img/thumbnail.lif`, `img/empty.lif`, `sounds/title.mp3`,
//!   `sounds/silent.mp3`) : mêmes suffixes `_2`, `_3`… que Python ;
//! - coût linéaire : la recherche du premier suffixe libre reprend où la précédente s'est
//!   arrêtée pour la même base (Python repart de `_2`, avec le même résultat) ;
//! - aucune panique ; le jeton d'annulation est consulté avant chaque asset et pendant le
//!   décodage et le redimensionnement des images ;
//! - progression comme Python : avertissements émis à l'instant où ils surviennent, une étape
//!   des phases `images` / `audio` par asset distinct traité (manquant ou illisible compris),
//!   aucune pour un asset déjà en cache.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Seek};

use crate::archive::Archive;
use crate::error::ConvError;
use crate::graph::{Assets, Warning};
use crate::image::{png_to_fit_lif, png_to_lif, png_to_scene_lif, Align, INV_H, INV_W};
use crate::mp3map;
use crate::png::PngError;
use crate::progress::{Phase, Progress};

/// Noms réservés par le convertisseur et le moteur (en minuscules).
const RESERVED: [&str; 5] = [
    "img/title.lif",
    "img/thumbnail.lif",
    "img/empty.lif",
    "sounds/title.mp3",
    "sounds/silent.mp3",
];

/// `os.path.splitext(os.path.basename(name))[0]` (le nom a passé `check_asset_name` : ni `\`
/// ni `:`, donc `ntpath` et `posixpath` coïncident). Les points de tête ne marquent pas une
/// extension (`.png` reste `.png`).
pub fn base_no_ext(name: &str) -> &str {
    let base = name.rsplit('/').next().unwrap_or(name);
    match base.rfind('.') {
        Some(dot) if base[..dot].bytes().any(|b| b != b'.') => &base[..dot],
        _ => base,
    }
}

/// `re.sub(r"[^A-Za-z0-9_-]+", "_", base).strip("_") or "asset"`.
pub fn sanitize(base: &str) -> String {
    let mut out = String::with_capacity(base.len());
    let mut in_run = false;
    for c in base.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
            in_run = false;
        } else if !in_run {
            out.push('_');
            in_run = true;
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "asset".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Espace de noms des sorties, insensible à la casse (`used` et `alloc` de Python).
#[derive(Debug)]
pub struct Names {
    used: HashSet<String>,
    /// Premier suffixe encore à essayer, par `(préfixe, base en minuscules, extension)`.
    next: HashMap<(String, String, String), u64>,
}

impl Default for Names {
    fn default() -> Self {
        Names {
            used: RESERVED.iter().map(|s| (*s).to_owned()).collect(),
            next: HashMap::new(),
        }
    }
}

impl Names {
    /// `alloc(prefix, base, ext)` : nom canonique unique dans `prefix`, sans le préfixe.
    pub fn alloc(&mut self, prefix: &str, base: &str, ext: &str) -> String {
        let b = sanitize(base);
        // Noms ASCII : `str.lower()` de Python est `to_ascii_lowercase`.
        let lower = |cand: &str| format!("{prefix}{cand}{ext}").to_ascii_lowercase();
        let mut cand = b.clone();
        if self.used.contains(&lower(&cand)) {
            let key = (prefix.to_owned(), b.to_ascii_lowercase(), ext.to_owned());
            let i = self.next.entry(key).or_insert(2);
            loop {
                cand = format!("{b}_{i}");
                *i += 1;
                if !self.used.contains(&lower(&cand)) {
                    break;
                }
            }
        }
        self.used.insert(lower(&cand));
        cand + ext
    }
}

/// Asset référencé mais absent de l'archive (`missing` de Python).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    /// `image` ou `audio`.
    pub kind: &'static str,
    pub name: String,
}

/// Résolveur des assets du graphe : implémente [`Assets`] sur une archive ouverte.
pub struct StoryAssets<'a, R> {
    archive: &'a mut Archive<R>,
    root: &'a str,
    keep_size: bool,
    cancelled: &'a dyn Fn() -> bool,
    pub(crate) progress: &'a mut dyn Progress,
    pub names: Names,
    img_cache: HashMap<(String, bool), Option<String>>,
    audio_cache: HashMap<String, String>,
    /// Fichiers produits : nom dans le paquet → octets (`out_files`).
    pub out: BTreeMap<String, Vec<u8>>,
    pub warnings: Vec<Warning>,
    pub missing: Vec<Missing>,
}

impl<'a, R: Read + Seek> StoryAssets<'a, R> {
    pub fn new(
        archive: &'a mut Archive<R>,
        root: &'a str,
        keep_size: bool,
        cancelled: &'a dyn Fn() -> bool,
        progress: &'a mut dyn Progress,
    ) -> Self {
        StoryAssets {
            archive,
            root,
            keep_size,
            cancelled,
            progress,
            names: Names::default(),
            img_cache: HashMap::new(),
            audio_cache: HashMap::new(),
            out: BTreeMap::new(),
            warnings: Vec::new(),
            missing: Vec::new(),
        }
    }

    /// Avertissement : émis tout de suite (`_warn`) et gardé dans [`Self::warnings`].
    pub fn warn(&mut self, code: &'static str, message: String) {
        self.progress.warn(code, &message);
        self.warnings.push(Warning { code, message });
    }

    /// Progression de la conversion (phases suivantes, après le graphe).
    pub fn progress(&mut self) -> &mut dyn Progress {
        &mut *self.progress
    }

    fn check_cancel(&self) -> Result<(), ConvError> {
        if (self.cancelled)() {
            Err(ConvError::cancelled())
        } else {
            Ok(())
        }
    }

    /// `read(rel)` de `_convert` : nom exact sous la racine, sinon recherche repliée.
    pub fn read(&mut self, rel: &str) -> Result<Option<Vec<u8>>, ConvError> {
        let Some(full) = self.archive.lookup(&format!("{}{rel}", self.root)) else {
            return Ok(None);
        };
        match self.archive.read(&full)? {
            Some(data) => Ok(Some(data)),
            None => Err(ConvError::internal_error(format!(
                "{full} : entree listee mais introuvable"
            ))),
        }
    }

    /// `add_mp3map` : table de seek, avertissement si la fréquence n'est pas 44 100 Hz,
    /// `mp3map_failed` si aucune trame n'est valide. Python rééchantillonne alors avec ffmpeg
    /// s'il le trouve ; le port recopie le MP3 tel quel, comme Python sans ffmpeg
    /// (divergence assumée, rééchantillonnage reporté : #22, #274).
    pub fn add_mp3map(&mut self, pk_name: &str, data: &[u8], label: &str) {
        match mp3map::build(data) {
            Ok(map) => {
                self.out.insert(format!("{pk_name}map"), map.data);
                let other: Vec<String> = map
                    .samplerates
                    .iter()
                    .filter(|&&sr| sr != 44100)
                    .map(u32::to_string)
                    .collect();
                if !other.is_empty() {
                    self.warn(
                        "audio_samplerate",
                        format!(
                            "{label} : {} Hz (le firmware est calibre pour 44100 Hz ; \
                             reechantillonnage impossible : non porte en Rust (#22) ; \
                             fichier recopie tel quel, reechantillonner conseille)",
                            other.join("/")
                        ),
                    );
                }
            }
            Err(e) => self.warn("mp3map_failed", format!("mp3map echoue pour {label} : {e}")),
        }
    }
}

/// Erreur d'image hors `image_unreadable` : délégation à Python ou annulation.
pub fn image_error(name: &str, e: PngError) -> ConvError {
    match e {
        PngError::Cancelled => ConvError::cancelled(),
        other => ConvError::unsupported_format(format!("{name} : {other}")),
    }
}

impl<R: Read + Seek> Assets for StoryAssets<'_, R> {
    fn image(
        &mut self,
        name: &str,
        _what: &str,
        inventory: bool,
    ) -> Result<Option<String>, ConvError> {
        self.check_cancel()?;
        let key = (name.to_owned(), inventory);
        if let Some(cached) = self.img_cache.get(&key) {
            return Ok(cached.clone());
        }
        let Some(data) = self.read(&format!("images/{name}"))? else {
            self.warn("image_missing", format!("image manquante: {name}"));
            self.missing.push(Missing {
                kind: "image",
                name: name.to_owned(),
            });
            self.progress.tick(Phase::Images);
            self.img_cache.insert(key, None);
            return Ok(None);
        };
        let cancelled = self.cancelled;
        let converted = if self.keep_size {
            png_to_lif(&data, cancelled)
        } else if inventory {
            png_to_fit_lif(&data, INV_W, INV_H, Align::Center, cancelled)
        } else {
            png_to_scene_lif(&data, cancelled)
        };
        drop(data);
        let lif = match converted {
            Ok(lif) => lif,
            Err(PngError::Unreadable(why)) => {
                self.warn(
                    "image_unreadable",
                    format!("image illisible {name} ({why}) -> empty.lif"),
                );
                let out = Some("empty.lif".to_owned());
                self.img_cache.insert(key, out.clone());
                self.progress.tick(Phase::Images);
                return Ok(out);
            }
            Err(e) => return Err(image_error(name, e)),
        };
        let suffix = if inventory { "_inv" } else { "" };
        let base = format!("{}{suffix}", base_no_ext(name));
        let lif_name = self.names.alloc("img/", &base, ".lif");
        self.out.insert(format!("img/{lif_name}"), lif);
        self.img_cache.insert(key, Some(lif_name.clone()));
        self.progress.tick(Phase::Images);
        Ok(Some(lif_name))
    }

    fn audio(&mut self, name: &str, _what: &str) -> Result<Option<String>, ConvError> {
        self.check_cancel()?;
        if let Some(cached) = self.audio_cache.get(name) {
            return Ok(Some(cached.clone()));
        }
        let Some(data) = self.read(&format!("audios/{name}"))? else {
            // Repli sur silent.mp3 : la scène garde son audio (et sa transition de fin).
            self.warn(
                "audio_missing",
                format!("audio manquant: {name} -> silent.mp3"),
            );
            self.missing.push(Missing {
                kind: "audio",
                name: name.to_owned(),
            });
            self.audio_cache
                .insert(name.to_owned(), "silent.mp3".to_owned());
            self.progress.tick(Phase::Audio);
            return Ok(Some("silent.mp3".to_owned()));
        };
        let out_name = self.names.alloc("sounds/", base_no_ext(name), ".mp3");
        let pk_name = format!("sounds/{out_name}");
        self.add_mp3map(&pk_name, &data, name);
        self.out.insert(pk_name, data);
        self.audio_cache.insert(name.to_owned(), out_name.clone());
        self.progress.tick(Phase::Audio);
        Ok(Some(out_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_sans_extension_comme_splitext() {
        for (name, base) in [
            ("s0.png", "s0"),
            ("dir/s0.png", "s0"),
            ("a.b.png", "a.b"),
            (".png", ".png"),
            ("..png", "..png"),
            ("dir/.x.png", ".x"),
            ("noext", "noext"),
            ("a.", "a"),
            ("Scène 1.png", "Scène 1"),
        ] {
            assert_eq!(base_no_ext(name), base, "{name}");
        }
    }

    #[test]
    fn nettoyage_comme_re_sub() {
        for (base, clean) in [
            ("s0", "s0"),
            ("Scène 1 #x", "Sc_ne_1_x"),
            ("__a__", "a"),
            ("é", "asset"),
            ("", "asset"),
            ("a--b", "a--b"),
            ("日本 語", "asset"),
            ("x日本y", "x_y"),
        ] {
            assert_eq!(sanitize(base), clean, "{base:?}");
        }
    }

    #[test]
    fn noms_uniques_insensibles_a_la_casse() {
        let mut n = Names::default();
        assert_eq!(n.alloc("img/", "title", ".lif"), "title_2.lif");
        assert_eq!(n.alloc("img/", "S0", ".lif"), "S0.lif");
        assert_eq!(n.alloc("img/", "s0", ".lif"), "s0_2.lif");
        assert_eq!(n.alloc("img/", "s0_3", ".lif"), "s0_3.lif");
        assert_eq!(n.alloc("img/", "s0!", ".lif"), "s0_4.lif");
        assert_eq!(n.alloc("sounds/", "s0", ".mp3"), "s0.mp3");
        assert_eq!(n.alloc("sounds/", "silent", ".mp3"), "silent_2.mp3");
        assert_eq!(n.alloc("img/", "S0", ".lif"), "S0_5.lif");
        assert_eq!(n.alloc("img/", "s0_2", ".lif"), "s0_2_2.lif");
    }

    /// Même résultat que la boucle de Python (`while … in used`), qui repart de `_2`.
    #[test]
    fn suffixes_identiques_a_la_boucle_naive() {
        fn naive(used: &mut HashSet<String>, prefix: &str, base: &str, ext: &str) -> String {
            let b = sanitize(base);
            let mut cand = b.clone();
            let mut i = 2;
            while used.contains(&format!("{prefix}{cand}{ext}").to_ascii_lowercase()) {
                cand = format!("{b}_{i}");
                i += 1;
            }
            used.insert(format!("{prefix}{cand}{ext}").to_ascii_lowercase());
            cand + ext
        }
        let bases = [
            "a", "A", "a_2", "a_3", "a_2_2", "b", "a!", "title", "a__3", "a_10",
        ];
        let mut used: HashSet<String> = RESERVED.iter().map(|s| (*s).to_owned()).collect();
        let mut n = Names::default();
        let mut seed: u32 = 7;
        for _ in 0..2000 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let base = bases[(seed >> 16) as usize % bases.len()];
            let prefix = if seed & 1 == 0 { "img/" } else { "sounds/" };
            assert_eq!(
                n.alloc(prefix, base, ".x"),
                naive(&mut used, prefix, base, ".x")
            );
        }
    }

    #[test]
    fn cout_lineaire_pour_une_meme_base() {
        let mut n = Names::default();
        for i in 0..100_000u32 {
            let got = n.alloc("img/", "a", ".lif");
            if i == 99_999 {
                assert_eq!(got, "a_100000.lif");
            }
        }
    }
}
