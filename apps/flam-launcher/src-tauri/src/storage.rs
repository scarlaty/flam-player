//! Persistance JSON versionnée : écriture atomique, sauvegarde des fichiers corrompus,
//! migration de schéma. Aucune dépendance à Tauri : tout est testable sur un dossier temporaire.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::{AppError, AppResult};

/// Nom du dossier de l'application sous `%APPDATA%` et `%LOCALAPPDATA%`.
const APP_DIR: &str = "flam-launcher";

/// Dossier de configuration (`%APPDATA%/flam-launcher` sous Windows).
pub fn config_dir() -> AppResult<PathBuf> {
    dirs::config_dir()
        .map(|dir| dir.join(APP_DIR))
        .ok_or(AppError::ConfigDir)
}

/// Dossier de cache par défaut (sous les données locales de l'application).
pub fn default_cache_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_DIR)
        .join("cache")
}

/// État Tauri partagé : dossier de configuration et verrou sérialisant les
/// cycles lecture-modification-écriture des commandes.
pub struct ConfigState {
    pub dir: PathBuf,
    lock: Mutex<()>,
}

impl ConfigState {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            lock: Mutex::new(()),
        }
    }

    /// Prend le verrou (un verrou empoisonné reste utilisable : les fichiers sont atomiques).
    pub fn guard(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Écrit `contents` dans `path` de façon atomique : fichier temporaire voisin, puis renommage.
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("chemin sans dossier parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("chemin sans nom de fichier"))?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".{}.tmp", name.to_string_lossy()));
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// Sérialise `value` en JSON indenté et l'écrit de façon atomique.
pub fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> AppResult<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)?;
    Ok(())
}

/// Horodatage UTC `AAAAMMJJ-HHMMSS` à partir d'un nombre de secondes depuis l'époque Unix.
pub fn utc_stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Conversion jours -> date civile (algorithme de H. Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

/// Renomme un fichier corrompu en `<nom>.<horodatage>.bak` (suffixe numérique si déjà pris).
pub fn backup_corrupted(path: &Path) -> io::Result<PathBuf> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let stamp = utc_stamp(secs);
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("chemin sans nom de fichier"))?
        .to_string_lossy()
        .into_owned();
    let mut backup = path.with_file_name(format!("{name}.{stamp}.bak"));
    let mut n = 1;
    while backup.exists() {
        backup = path.with_file_name(format!("{name}.{stamp}-{n}.bak"));
        n += 1;
    }
    fs::rename(path, &backup)?;
    Ok(backup)
}

/// Étape de migration : fait passer l'objet de la version `from` à `from + 1`.
pub type MigrationStep = fn(from: u32, object: &mut Map<String, Value>);

/// Lit et migre un fichier JSON versionné (champ `schema_version`, absent = version 0).
///
/// - fichier absent : `Ok(None)` ;
/// - fichier illisible, JSON invalide ou de forme inattendue : sauvegardé en `*.bak`,
///   avertissement tracé, `Ok(None)` (l'appelant retombe sur les valeurs par défaut) ;
/// - version de schéma plus récente que `current` : `Err(ConfigVersion)`, fichier intact.
pub fn load_versioned<T: DeserializeOwned>(
    path: &Path,
    current: u32,
    step: MigrationStep,
) -> AppResult<Option<T>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        // Contenu non UTF-8 : fichier corrompu.
        Err(err) if err.kind() == io::ErrorKind::InvalidData => {
            return quarantine(path, &err.to_string());
        }
        Err(err) => return Err(err.into()),
    };
    let mut object = match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(object)) => object,
        Ok(_) => return quarantine(path, "la racine n'est pas un objet JSON"),
        Err(err) => return quarantine(path, &err.to_string()),
    };
    let found = match object.get("schema_version") {
        None => 0,
        Some(value) => match value.as_u64() {
            Some(version) => version,
            None => return quarantine(path, "schema_version n'est pas un entier"),
        },
    };
    if found > u64::from(current) {
        return Err(AppError::ConfigVersion {
            file: path.display().to_string(),
            found,
            supported: current,
        });
    }
    for from in found as u32..current {
        step(from, &mut object);
    }
    object.insert("schema_version".into(), Value::from(current));
    match serde_json::from_value(Value::Object(object)) {
        Ok(value) => Ok(Some(value)),
        Err(err) => quarantine(path, &err.to_string()),
    }
}

fn quarantine<T>(path: &Path, reason: &str) -> AppResult<Option<T>> {
    let backup = backup_corrupted(path)?;
    tracing::warn!(
        "fichier de configuration corrompu ({reason}) : {} sauvegardé en {}, valeurs par défaut utilisées",
        path.display(),
        backup.display()
    );
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Deserialize)]
    struct Doc {
        schema_version: u32,
        #[serde(default)]
        value: String,
    }

    fn noop(_: u32, _: &mut Map<String, Value>) {}

    fn rename_old(from: u32, object: &mut Map<String, Value>) {
        if from == 1 {
            if let Some(old) = object.remove("old") {
                object.insert("value".into(), old);
            }
        }
    }

    fn file_names(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn utc_stamp_formate_une_date_connue() {
        assert_eq!(utc_stamp(0), "19700101-000000");
        assert_eq!(utc_stamp(1_700_000_000), "20231114-221320");
        // 29 février 2024, 23:59:59.
        assert_eq!(utc_stamp(1_709_251_199), "20240229-235959");
    }

    #[test]
    fn write_atomic_cree_les_dossiers_et_remplace_le_fichier() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/f.json");
        write_atomic(&path, b"un").unwrap();
        write_atomic(&path, b"deux").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"deux");
        assert_eq!(
            file_names(path.parent().unwrap()),
            ["f.json"],
            "pas de fichier temporaire résiduel"
        );
    }

    #[test]
    fn fichier_absent_donne_none() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_versioned::<Doc>(&dir.path().join("x.json"), 1, noop).unwrap();
        assert_eq!(loaded, None);
    }

    #[test]
    fn fichier_valide_est_lu() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        fs::write(&path, r#"{"schema_version":1,"value":"v"}"#).unwrap();
        let doc = load_versioned::<Doc>(&path, 1, noop).unwrap().unwrap();
        assert_eq!(doc.value, "v");
    }

    #[test]
    fn json_invalide_est_sauvegarde_en_bak() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        fs::write(&path, "{pas du json").unwrap();
        assert_eq!(load_versioned::<Doc>(&path, 1, noop).unwrap(), None);
        assert!(!path.exists());
        let names = file_names(dir.path());
        assert_eq!(names.len(), 1);
        assert!(names[0].starts_with("x.json.") && names[0].ends_with(".bak"));
        let content = fs::read_to_string(dir.path().join(&names[0])).unwrap();
        assert_eq!(content, "{pas du json");
    }

    #[test]
    fn plusieurs_sauvegardes_ne_se_collisionnent_pas() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        for _ in 0..3 {
            fs::write(&path, "[").unwrap();
            assert_eq!(load_versioned::<Doc>(&path, 1, noop).unwrap(), None);
        }
        assert_eq!(file_names(dir.path()).len(), 3);
    }

    #[test]
    fn racine_non_objet_et_mauvais_types_sont_corrompus() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        for bad in ["[1]", r#"{"schema_version":"1"}"#, r#"{"value":3}"#] {
            fs::write(&path, bad).unwrap();
            assert_eq!(
                load_versioned::<Doc>(&path, 1, noop).unwrap(),
                None,
                "{bad}"
            );
            assert!(!path.exists());
        }
    }

    #[test]
    fn contenu_non_utf8_est_corrompu() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(load_versioned::<Doc>(&path, 1, noop).unwrap(), None);
        assert!(!path.exists());
    }

    #[test]
    fn version_absente_est_migree_depuis_zero() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        fs::write(&path, r#"{"value":"v"}"#).unwrap();
        let doc = load_versioned::<Doc>(&path, 1, noop).unwrap().unwrap();
        assert_eq!(doc.schema_version, 1);
    }

    #[test]
    fn les_etapes_de_migration_sont_appliquees_dans_l_ordre() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        fs::write(&path, r#"{"schema_version":1,"old":"ancien"}"#).unwrap();
        let doc = load_versioned::<Doc>(&path, 2, rename_old)
            .unwrap()
            .unwrap();
        assert_eq!(
            doc,
            Doc {
                schema_version: 2,
                value: "ancien".into()
            }
        );
    }

    #[test]
    fn version_plus_recente_est_refusee_sans_toucher_au_fichier() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        fs::write(&path, r#"{"schema_version":7}"#).unwrap();
        let err = load_versioned::<Doc>(&path, 1, noop).unwrap_err();
        assert_eq!(err.kind(), "config_version");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            r#"{"schema_version":7}"#
        );
    }
}
