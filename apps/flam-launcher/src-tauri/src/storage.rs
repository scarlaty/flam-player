//! Persistance JSON versionnée : écriture atomique, sauvegarde des fichiers corrompus,
//! migration de schéma, mémorisation des restaurations. Aucune dépendance à Tauri : tout est
//! testable sur un dossier temporaire.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};
use tauri::State;

use crate::{AppError, AppResult};

/// Nom du dossier de l'application sous `%APPDATA%` et `%LOCALAPPDATA%`.
const APP_DIR: &str = "flam-launcher";
/// Fichier mémorisant les restaurations non encore vues par l'utilisateur.
const RECOVERY_FILE: &str = "recovery.json";
/// Tentatives de renommage en cas d'accès refusé (antivirus, indexeur) et délai entre elles.
const RENAME_ATTEMPTS: u32 = 5;
const RENAME_DELAY: Duration = Duration::from_millis(30);

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

/// Restauration aux valeurs par défaut d'un fichier de configuration corrompu
/// (miroir de `Recovery` dans `src/api/recovery.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Recovery {
    /// Fichier restauré (`settings.json`, `sources.json`).
    pub file: String,
    /// Nom de la sauvegarde `*.bak` créée à côté du fichier.
    pub backup: String,
}

/// Renomme avec quelques nouvelles tentatives si l'accès est refusé (antivirus, indexeur).
fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    let mut attempt = 1;
    loop {
        match fs::rename(from, to) {
            Err(err)
                if err.kind() == io::ErrorKind::PermissionDenied && attempt < RENAME_ATTEMPTS =>
            {
                attempt += 1;
                thread::sleep(RENAME_DELAY);
            }
            other => return other,
        }
    }
}

/// Écrit `contents` dans `path` de façon atomique : fichier temporaire voisin (nom unique
/// par processus et par appel), puis renommage.
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("chemin sans dossier parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("chemin sans nom de fichier"))?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}.{}-{}.tmp",
        name.to_string_lossy(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        rename_with_retry(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Sérialise `value` en JSON indenté et l'écrit de façon atomique.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> AppResult<()> {
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
    rename_with_retry(path, &backup)?;
    Ok(backup)
}

/// Étape de migration : fait passer l'objet de la version `from` à `from + 1`.
pub type MigrationStep = fn(from: u32, object: &mut Map<String, Value>);

/// Résultat de la lecture d'un fichier versionné.
#[derive(Debug, PartialEq)]
pub enum Loaded<T> {
    /// Fichier absent.
    Missing,
    /// Fichier lu (et migré si besoin).
    Found(T),
    /// Fichier corrompu : renommé en `backup` (nom de fichier).
    Recovered { backup: String },
}

/// Lit et migre un fichier JSON versionné (champ `schema_version`, absent = version 0).
///
/// - fichier absent : `Loaded::Missing` ;
/// - fichier illisible, JSON invalide ou de forme inattendue : sauvegardé en `*.bak`,
///   avertissement tracé, `Loaded::Recovered` ;
/// - version de schéma plus récente que `current` : `Err(ConfigVersion)`, fichier intact.
pub fn load_versioned<T: DeserializeOwned>(
    path: &Path,
    current: u32,
    step: MigrationStep,
) -> AppResult<Loaded<T>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Loaded::Missing),
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
        Ok(value) => Ok(Loaded::Found(value)),
        Err(err) => quarantine(path, &err.to_string()),
    }
}

fn quarantine<T>(path: &Path, reason: &str) -> AppResult<Loaded<T>> {
    let backup = backup_corrupted(path)?;
    tracing::warn!(
        "fichier de configuration corrompu ({reason}) : {} sauvegardé en {}, valeurs par défaut utilisées",
        path.display(),
        backup.display()
    );
    let backup = backup
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(Loaded::Recovered { backup })
}

/// Charge `<dir>/<file_name>`. Fichier absent : valeurs par défaut écrites sur disque.
/// Fichier corrompu : sauvegarde `*.bak`, valeurs par défaut écrites, restauration mémorisée
/// (voir [`pending_recovery`]). Version de schéma future : erreur, fichier intact.
pub fn load_or_default<T>(
    dir: &Path,
    file_name: &str,
    current: u32,
    step: MigrationStep,
) -> AppResult<T>
where
    T: DeserializeOwned + Serialize + Default,
{
    let path = dir.join(file_name);
    match load_versioned(&path, current, step)? {
        Loaded::Found(value) => Ok(value),
        Loaded::Missing => {
            let value = T::default();
            write_json(&path, &value)?;
            Ok(value)
        }
        Loaded::Recovered { backup } => {
            let value = T::default();
            write_json(&path, &value)?;
            record_recovery(dir, file_name, &backup)?;
            Ok(value)
        }
    }
}

fn read_recovery_map(dir: &Path) -> BTreeMap<String, String> {
    fs::read_to_string(dir.join(RECOVERY_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn record_recovery(dir: &Path, file: &str, backup: &str) -> AppResult<()> {
    let mut map = read_recovery_map(dir);
    map.insert(file.to_string(), backup.to_string());
    write_json(&dir.join(RECOVERY_FILE), &map)
}

/// Restauration non encore acquittée pour `file`, s'il y en a une. Elle reste signalée
/// jusqu'à [`ack_recovery`] (même après un redémarrage).
pub fn pending_recovery(dir: &Path, file: &str) -> Option<Recovery> {
    read_recovery_map(dir).remove(file).map(|backup| Recovery {
        file: file.to_string(),
        backup,
    })
}

/// Acquitte la restauration de `file` (ou toutes si `None`).
pub fn ack_recovery(dir: &Path, file: Option<&str>) -> AppResult<()> {
    let mut map = read_recovery_map(dir);
    match file {
        Some(file) => {
            map.remove(file);
        }
        None => map.clear(),
    }
    if map.is_empty() {
        match fs::remove_file(dir.join(RECOVERY_FILE)) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err.into()),
            _ => Ok(()),
        }
    } else {
        write_json(&dir.join(RECOVERY_FILE), &map)
    }
}

/// Acquitte l'information de restauration d'un fichier (`settings.json` ou `sources.json`),
/// ou de tous si `file` est absent. À appeler quand l'utilisateur a vu l'avertissement.
#[tauri::command]
pub async fn config_ack_recovery(
    state: State<'_, ConfigState>,
    file: Option<String>,
) -> AppResult<()> {
    let _guard = state.guard();
    ack_recovery(&state.dir, file.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    struct Doc {
        schema_version: u32,
        #[serde(default)]
        value: String,
    }

    impl Default for Doc {
        fn default() -> Self {
            Doc {
                schema_version: 1,
                value: "défaut".into(),
            }
        }
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

    fn is_recovered(loaded: AppResult<Loaded<Doc>>) -> bool {
        matches!(loaded.unwrap(), Loaded::Recovered { .. })
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
    fn ecritures_concurrentes_sans_collision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.json");
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                thread::spawn(move || {
                    for _ in 0..20 {
                        write_atomic(&path, format!("{{\"w\":{i}}}").as_bytes()).unwrap();
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let content = fs::read_to_string(&path).unwrap();
        assert!(serde_json::from_str::<Value>(&content).is_ok());
        assert_eq!(file_names(dir.path()), ["f.json"]);
    }

    #[test]
    fn fichier_absent_donne_missing() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_versioned::<Doc>(&dir.path().join("x.json"), 1, noop).unwrap();
        assert_eq!(loaded, Loaded::Missing);
    }

    #[test]
    fn fichier_valide_est_lu() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        fs::write(&path, r#"{"schema_version":1,"value":"v"}"#).unwrap();
        match load_versioned::<Doc>(&path, 1, noop).unwrap() {
            Loaded::Found(doc) => assert_eq!(doc.value, "v"),
            other => panic!("inattendu : {other:?}"),
        }
    }

    #[test]
    fn json_invalide_est_sauvegarde_en_bak() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        fs::write(&path, "{pas du json").unwrap();
        let loaded = load_versioned::<Doc>(&path, 1, noop).unwrap();
        assert!(!path.exists());
        let names = file_names(dir.path());
        assert_eq!(names.len(), 1);
        assert!(names[0].starts_with("x.json.") && names[0].ends_with(".bak"));
        assert_eq!(
            loaded,
            Loaded::Recovered {
                backup: names[0].clone()
            }
        );
        let content = fs::read_to_string(dir.path().join(&names[0])).unwrap();
        assert_eq!(content, "{pas du json");
    }

    #[test]
    fn plusieurs_sauvegardes_ne_se_collisionnent_pas() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        for _ in 0..3 {
            fs::write(&path, "[").unwrap();
            assert!(is_recovered(load_versioned::<Doc>(&path, 1, noop)));
        }
        assert_eq!(file_names(dir.path()).len(), 3);
    }

    #[test]
    fn racine_non_objet_et_mauvais_types_sont_corrompus() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        for bad in ["[1]", r#"{"schema_version":"1"}"#, r#"{"value":3}"#] {
            fs::write(&path, bad).unwrap();
            assert!(is_recovered(load_versioned::<Doc>(&path, 1, noop)), "{bad}");
            assert!(!path.exists());
        }
    }

    #[test]
    fn contenu_non_utf8_est_corrompu() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert!(is_recovered(load_versioned::<Doc>(&path, 1, noop)));
        assert!(!path.exists());
    }

    #[test]
    fn version_absente_est_migree_depuis_zero() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        fs::write(&path, r#"{"value":"v"}"#).unwrap();
        match load_versioned::<Doc>(&path, 1, noop).unwrap() {
            Loaded::Found(doc) => assert_eq!(doc.schema_version, 1),
            other => panic!("inattendu : {other:?}"),
        }
    }

    #[test]
    fn les_etapes_de_migration_sont_appliquees_dans_l_ordre() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        fs::write(&path, r#"{"schema_version":1,"old":"ancien"}"#).unwrap();
        let loaded = load_versioned::<Doc>(&path, 2, rename_old).unwrap();
        assert_eq!(
            loaded,
            Loaded::Found(Doc {
                schema_version: 2,
                value: "ancien".into()
            })
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

    #[test]
    fn load_or_default_ecrit_les_valeurs_par_defaut_sans_restauration() {
        let dir = tempfile::tempdir().unwrap();
        let doc: Doc = load_or_default(dir.path(), "x.json", 1, noop).unwrap();
        assert_eq!(doc, Doc::default());
        assert!(dir.path().join("x.json").exists());
        assert_eq!(pending_recovery(dir.path(), "x.json"), None);
    }

    #[test]
    fn la_restauration_est_memorisee_jusqu_a_l_acquittement() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("x.json"), "oups").unwrap();
        fs::write(dir.path().join("y.json"), "oups").unwrap();
        let doc: Doc = load_or_default(dir.path(), "x.json", 1, noop).unwrap();
        let _: Doc = load_or_default(dir.path(), "y.json", 1, noop).unwrap();
        assert_eq!(doc, Doc::default());

        let recovery = pending_recovery(dir.path(), "x.json").unwrap();
        assert_eq!(recovery.file, "x.json");
        assert!(recovery.backup.starts_with("x.json.") && recovery.backup.ends_with(".bak"));
        assert!(dir.path().join(&recovery.backup).exists());

        // Relecture normale : l'information persiste.
        let _: Doc = load_or_default(dir.path(), "x.json", 1, noop).unwrap();
        assert_eq!(pending_recovery(dir.path(), "x.json"), Some(recovery));

        ack_recovery(dir.path(), Some("x.json")).unwrap();
        assert_eq!(pending_recovery(dir.path(), "x.json"), None);
        assert!(pending_recovery(dir.path(), "y.json").is_some());
        ack_recovery(dir.path(), None).unwrap();
        assert_eq!(pending_recovery(dir.path(), "y.json"), None);
        assert!(!dir.path().join("recovery.json").exists());
        // Acquitter sans rien à acquitter n'est pas une erreur.
        ack_recovery(dir.path(), None).unwrap();
    }

    #[test]
    fn un_fichier_de_restauration_illisible_est_ignore() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("recovery.json"), "pas du json").unwrap();
        assert_eq!(pending_recovery(dir.path(), "x.json"), None);
    }
}
