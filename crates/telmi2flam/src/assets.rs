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
//! - aucune panique ; le jeton d'annulation est consulté avant chaque asset, pendant le
//!   décodage et le redimensionnement des images et pendant ffmpeg ;
//! - un MP3 hors 44 100 Hz n'est remplacé que par une sortie de ffmpeg ([`crate::ffmpeg`])
//!   validée par `mp3map::build` (44 100 Hz seulement) ; sinon il est recopié tel quel, avec
//!   l'avertissement `audio_samplerate` et la raison de Python. Un même contenu sous plusieurs
//!   noms ne lance ffmpeg qu'une fois (cache par contenu exact ; Python : SHA-256).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Seek};

use crate::archive::Archive;
use crate::error::ConvError;
use crate::ffmpeg::{self, Outcome, SAMPLERATE};
use crate::graph::{Assets, Warning};
use crate::image::{png_to_fit_lif, png_to_lif, png_to_scene_lif, Align, INV_H, INV_W};
use crate::mp3map;
use crate::png::PngError;

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
    pub names: Names,
    img_cache: HashMap<(String, bool), Option<String>>,
    audio_cache: HashMap<String, String>,
    /// Fichiers produits : nom dans le paquet → octets (`out_files`).
    pub out: BTreeMap<String, Vec<u8>>,
    pub warnings: Vec<Warning>,
    pub missing: Vec<Missing>,
    /// Sorties de ffmpeg par contenu source (`resample_cache` de Python, clé SHA-256).
    resample_cache: HashMap<Vec<u8>, Outcome>,
    /// Fichiers rééchantillonnés (`stats["resampled"]`, compté par fichier, cache compris).
    pub resampled: usize,
    /// Lignes d'information `  i …` de Python (rééchantillonnage réussi), sans le préfixe.
    /// Pas encore affichées ni émises en progression (#131).
    pub infos: Vec<String>,
}

impl<'a, R: Read + Seek> StoryAssets<'a, R> {
    pub fn new(
        archive: &'a mut Archive<R>,
        root: &'a str,
        keep_size: bool,
        cancelled: &'a dyn Fn() -> bool,
    ) -> Self {
        StoryAssets {
            archive,
            root,
            keep_size,
            cancelled,
            names: Names::default(),
            img_cache: HashMap::new(),
            audio_cache: HashMap::new(),
            out: BTreeMap::new(),
            warnings: Vec::new(),
            missing: Vec::new(),
            resample_cache: HashMap::new(),
            resampled: 0,
            infos: Vec::new(),
        }
    }

    pub fn warn(&mut self, code: &'static str, message: String) {
        self.warnings.push(Warning { code, message });
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

    /// Rééchantillonnage de `data` par ffmpeg, une fois par contenu (`resample_cache`).
    fn resample(&mut self, data: &[u8], duration: f64) -> Result<Outcome, ConvError> {
        if let Some(cached) = self.resample_cache.get(data) {
            return Ok(cached.clone());
        }
        let outcome = ffmpeg::resample_mp3(data, duration, self.cancelled)?;
        self.resample_cache.insert(data.to_vec(), outcome.clone());
        Ok(outcome)
    }

    /// `add_mp3map` : écrit `pk_name` et sa table de seek. Un MP3 hors 44 100 Hz est d'abord
    /// rééchantillonné par ffmpeg ([`crate::ffmpeg`]) ; la sortie, si `mp3map::build` la lit
    /// et n'y voit que 44 100 Hz, remplace le fichier. Sinon avertissement `audio_samplerate`
    /// avec la raison, et MP3 recopié tel quel. `mp3map_failed` si aucune trame n'est valide
    /// (MP3 recopié, sans table). `Err` seulement sur annulation pendant ffmpeg.
    pub fn add_mp3map(
        &mut self,
        pk_name: &str,
        mut data: Vec<u8>,
        label: &str,
    ) -> Result<(), ConvError> {
        match mp3map::build(&data) {
            Ok(mut map) => {
                let other: Vec<String> = map
                    .samplerates
                    .iter()
                    .filter(|&&sr| sr != SAMPLERATE)
                    .map(u32::to_string)
                    .collect();
                if !other.is_empty() {
                    let rates = other.join("/");
                    let failure = match self.resample(&data, map.duration_s)? {
                        Ok(new) => match mp3map::build(&new) {
                            Ok(map2)
                                if map2.samplerates.len() == 1
                                    && map2.samplerates.contains(&SAMPLERATE) =>
                            {
                                data = new;
                                map = map2;
                                self.resampled += 1;
                                self.infos.push(format!(
                                    "{label} : {rates} Hz -> {SAMPLERATE} Hz \
                                     (reechantillonne avec ffmpeg)"
                                ));
                                None
                            }
                            _ => Some("sortie ffmpeg illisible ou inattendue".to_owned()),
                        },
                        Err(why) => Some(why),
                    };
                    if let Some(why) = failure {
                        self.warn(
                            "audio_samplerate",
                            format!(
                                "{label} : {rates} Hz (le firmware est calibre pour 44100 Hz ; \
                                 reechantillonnage impossible : {why} ; \
                                 fichier recopie tel quel, reechantillonner conseille)"
                            ),
                        );
                    }
                }
                self.out.insert(format!("{pk_name}map"), map.data);
            }
            Err(e) => self.warn("mp3map_failed", format!("mp3map echoue pour {label} : {e}")),
        }
        self.out.insert(pk_name.to_owned(), data);
        Ok(())
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
                return Ok(out);
            }
            Err(e) => return Err(image_error(name, e)),
        };
        let suffix = if inventory { "_inv" } else { "" };
        let base = format!("{}{suffix}", base_no_ext(name));
        let lif_name = self.names.alloc("img/", &base, ".lif");
        self.out.insert(format!("img/{lif_name}"), lif);
        self.img_cache.insert(key, Some(lif_name.clone()));
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
            return Ok(Some("silent.mp3".to_owned()));
        };
        let out_name = self.names.alloc("sounds/", base_no_ext(name), ".mp3");
        let pk_name = format!("sounds/{out_name}");
        self.add_mp3map(&pk_name, data, name)?;
        self.audio_cache.insert(name.to_owned(), out_name.clone());
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
