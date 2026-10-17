//! Moteur (`tools/telmi2flam/engine/`) et runtime Lunii (`tools/telmi2flam/runtime/`) inclus
//! dans le binaire (PORT-RUST.md §2.1) : plus de dossier de ressources à trouver à l'exécution.
//!
//! La liste du runtime vient du manifeste unique `runtime/manifest.txt`, lu aussi par
//! `telmi2flam.py` (fichiers indispensables) et `validate.py` ; `build.rs` en génère la table.
//! Les octets sont ceux de la copie de travail : `.gitattributes` force LF sur les `.lua` et
//! garde les `.lif` en binaire.

/// Fichier du runtime copié tel quel dans chaque paquet.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeFile {
    /// Nom dans le paquet (`script/x.lua` ou `img/script/x.lif`), identique au chemin relatif
    /// à `runtime/`.
    pub name: &'static str,
    /// Indispensable au démarrage (ligne `!` du manifeste : `RUNTIME_SCRIPTS` /
    /// `RUNTIME_UI_LIF` de Python).
    pub required: bool,
    pub data: &'static [u8],
}

/// Runtime dans l'ordre du manifeste.
pub static RUNTIME: &[RuntimeFile] = include!(concat!(env!("OUT_DIR"), "/runtime_files.rs"));

/// Bootstrap du paquet (`engine/main.lua`), à la racine du paquet.
pub use crate::graph::MAIN_LUA;

/// Moteur d'histoire (`engine/story.lua`), copié dans `script/story.lua`.
pub const STORY_LUA: &[u8] = include_bytes!("../../../tools/telmi2flam/engine/story.lua");

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn runtime_dir() -> &'static Path {
        Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/telmi2flam/runtime"
        ))
    }

    /// Même règle que `_convert` : tous les `.lua` de `runtime/script` et tous les `.lif` de
    /// `runtime/img/script`, triés.
    fn on_disk(dir: &str, ext: &str) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(runtime_dir().join(dir))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.ends_with(ext))
            .map(|n| format!("{dir}/{n}"))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn manifeste_identique_aux_dossiers() {
        let mut expected = on_disk("script", ".lua");
        expected.extend(on_disk("img/script", ".lif"));
        let listed: Vec<&str> = RUNTIME.iter().map(|f| f.name).collect();
        assert_eq!(listed, expected);
        for f in RUNTIME {
            let disk = std::fs::read(runtime_dir().join(f.name)).unwrap();
            assert_eq!(f.data, disk.as_slice(), "{}", f.name);
        }
    }

    #[test]
    fn fichiers_indispensables_de_python() {
        let required: Vec<&str> = RUNTIME
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert_eq!(required.len(), 17);
        for n in ["script/global.lua", "img/script/audio-player-pause.lif"] {
            assert!(required.contains(&n), "{n}");
        }
    }

    #[test]
    fn scripts_en_lf() {
        for f in RUNTIME.iter().filter(|f| f.name.ends_with(".lua")) {
            assert!(!f.data.contains(&b'\r'), "{} en CRLF", f.name);
        }
        assert!(!STORY_LUA.contains(&b'\r') && !MAIN_LUA.contains(&b'\r'));
    }
}
