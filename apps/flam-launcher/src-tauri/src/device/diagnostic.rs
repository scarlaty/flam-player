//! Diagnostic copiable de l'écran « Ma Flam » (#318) : un texte Markdown à coller dans une issue
//! « Retour testeur Flam ».
//!
//! **Lecture seule** : rien n'est écrit sur la Flam (ni reprise, ni sauvegarde de `.mdf`), aucun
//! accès réseau. Le texte reste utile sans Flam détectée (version du launcher, système, erreurs
//! de la session) et quand la garde refuse d'écrire (c'est là qu'il sert le plus).
//!
//! # Contenu
//!
//! Version du launcher, version de Windows, état du firmware ([`FirmwareStatus`] complet), état
//! des index `list` et `list.hidden` (nombres d'entrées, fins de ligne, UUID non canoniques,
//! entrées sans dossier, dossiers hors index), présence des fichiers de travail
//! (`.flam-transfer`, `.flam-trash`, `list.new`, `list.bak`, `list.corrupt*`…), de
//! `usr/0/library.cache` (taille), espace libre et total, nombre d'histoires, dernières erreurs de
//! transfert de la session.
//!
//! # Jamais
//!
//! Ni clé, ni IV, ni `bt`, ni numéro de série, ni empreinte de `.mdf`, ni nom de volume, ni
//! titre, contenu ou UUID d'histoire (des nombres seulement), ni nom d'utilisateur : le profil
//! Windows est remplacé par `%USERPROFILE%` dans les messages d'erreur. Par sécurité, le texte
//! final passe en plus par [`Redactor::scrub`] : tout UUID, toute suite d'au moins
//! [`MIN_HEX_RUN`] chiffres hexadécimaux et tout mot qui ressemble à du base64 sont masqués.

use std::collections::{HashSet, VecDeque};
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use tauri::State;

use super::detect::{self, Environment, MdfError, Space};
use super::guard::{self, Coherence, Family, FirmwareStatus, Trust, VersionForm, WriteContext};
use super::index::{self, IndexState, StoryId, LIBRARY_CACHE, LIBRARY_DIR, MAX_INDEX_BYTES};
use super::lock::DEVICE_LOCK;
use super::remove::TRASH_DIR;
use super::transfer::{HIDDEN_STORIES_DIR, TRANSFER_DIR};
use crate::catalog::blocking;
use crate::storage::ConfigState;
use crate::{AppError, AppResult};

/// Nombre d'erreurs de la session gardées (les plus anciennes sont oubliées).
pub const MAX_SESSION_ERRORS: usize = 10;
/// Longueur maximale (caractères) d'un message d'erreur gardé.
const MAX_ERROR_LEN: usize = 300;
/// Suite de chiffres hexadécimaux masquée à partir de cette longueur (numéro de série : 14).
pub const MIN_HEX_RUN: usize = 14;
/// Mot de type base64 masqué à partir de cette longueur (s'il mêle chiffres, majuscules et
/// minuscules).
const MIN_BASE64_RUN: usize = 16;
/// Nombre maximal d'entrées de dossier examinées par dossier (Flam abîmée : pas de balayage
/// sans fin).
const MAX_DIR_ENTRIES: usize = 10_000;
/// Remplacement d'une valeur masquée.
const MASK: &str = "<masqué>";

// --- Erreurs de la session --------------------------------------------------------------------

/// Erreur d'une opération sur la Flam, gardée en mémoire pour le diagnostic (jamais écrite sur
/// disque).
#[derive(Debug, Clone)]
pub struct SessionError {
    at: Instant,
    operation: &'static str,
    message: String,
}

static SESSION_ERRORS: Mutex<VecDeque<SessionError>> = Mutex::new(VecDeque::new());

/// Garde le message d'une erreur de `operation` (« envoi », « reprise ») pour le diagnostic.
/// Le message est nettoyé dès l'enregistrement ([`Redactor::current`]) et tronqué.
pub fn record_error(operation: &'static str, message: &str) {
    let message = Redactor::current().clean_message(message);
    let mut errors = SESSION_ERRORS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if errors.len() == MAX_SESSION_ERRORS {
        errors.pop_front();
    }
    errors.push_back(SessionError {
        at: Instant::now(),
        operation,
        message,
    });
}

/// Message d'une erreur de commande à garder pour le diagnostic. Liste blanche : seules les
/// erreurs de validation, d'entrée/sortie et « introuvable » (messages des couches de la Flam, sans
/// titre) sont reprises ; une autre variante (dont `StoryFileMissing`, qui cite le titre de
/// l'histoire) ne donne que son code.
pub fn session_message(err: &AppError) -> String {
    match err {
        AppError::Validation(_) | AppError::Io(_) | AppError::NotFound(_) => err.to_string(),
        AppError::StoryFileMissing(_) => {
            "fichier de l'histoire absent de la bibliothèque du launcher".to_string()
        }
        other => {
            let kind = serde_json::to_value(other)
                .ok()
                .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_string))
                .unwrap_or_else(|| "inconnue".into());
            format!("erreur de type « {kind} » (message non repris)")
        }
    }
}

/// Erreurs de la session, de la plus ancienne à la plus récente.
pub fn session_errors() -> Vec<SessionError> {
    SESSION_ERRORS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .cloned()
        .collect()
}

// --- Masquage ---------------------------------------------------------------------------------

/// Masquage des données personnelles d'un texte : profil Windows, nom d'utilisateur, UUID,
/// suites hexadécimales et mots de type base64.
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    /// Dossier du profil (`C:\Users\nom`), remplacé par `%USERPROFILE%`.
    pub profile: Option<String>,
    /// Nom d'utilisateur, remplacé par `%USERNAME%` dans les messages d'erreur.
    pub username: Option<String>,
}

impl Redactor {
    /// Profil et nom de l'utilisateur courant (variables `USERPROFILE` et `USERNAME`).
    pub fn current() -> Self {
        let var = |name| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Self {
            profile: var("USERPROFILE"),
            username: var("USERNAME"),
        }
    }

    /// Message d'erreur d'une seule ligne, sans profil ni nom d'utilisateur, tronqué à
    /// [`MAX_ERROR_LEN`] caractères, puis [`Self::scrub`].
    pub fn clean_message(&self, message: &str) -> String {
        let mut text: String = message
            .chars()
            .map(|c| if c.is_control() || c == '`' { ' ' } else { c })
            .collect();
        if let Some(profile) = &self.profile {
            let profile = profile.trim_end_matches(['\\', '/']);
            if profile.chars().count() >= 3 {
                text = replace_ci(&text, profile, "%USERPROFILE%");
                text = replace_ci(&text, &profile.replace('\\', "/"), "%USERPROFILE%");
            }
        }
        text = mask_users_segment(&text);
        if let Some(name) = &self.username {
            if name.chars().count() >= 3 {
                text = replace_word_ci(&text, name, "%USERNAME%");
            }
        }
        let text = self.scrub(&text);
        let mut out: String = text.chars().take(MAX_ERROR_LEN).collect();
        if text.chars().count() > MAX_ERROR_LEN {
            out.push('…');
        }
        out
    }

    /// Dernier filet, appliqué au texte entier : UUID (`<uuid>`), suites d'au moins
    /// [`MIN_HEX_RUN`] chiffres hexadécimaux et mots de type base64 ([`MASK`]), segment de
    /// `\Users\` des chemins.
    pub fn scrub(&self, text: &str) -> String {
        let text = mask_uuids(&mask_users_segment(text));
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        while i < chars.len() {
            if is_token_char(chars[i]) && (i == 0 || !is_token_char(chars[i - 1])) {
                let end = (i..chars.len())
                    .find(|&j| !is_token_char(chars[j]))
                    .unwrap_or(chars.len());
                let token: String = chars[i..end].iter().collect();
                out.push_str(&mask_token(&token));
                i = end;
                continue;
            }
            out.push(chars[i]);
            i += 1;
        }
        out
    }
}

/// Caractère d'un mot examiné par [`mask_token`] : alphabet base64 (standard et URL).
fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '-' | '_')
}

/// Masque dans `token` les suites hexadécimales longues, ou le mot entier s'il ressemble à du
/// base64.
fn mask_token(token: &str) -> String {
    let looks_base64 = token.len() >= MIN_BASE64_RUN
        && token.bytes().any(|b| b.is_ascii_digit())
        && token.bytes().any(|b| b.is_ascii_uppercase())
        && token.bytes().any(|b| b.is_ascii_lowercase());
    if looks_base64 {
        return MASK.to_string();
    }
    let mut out = String::with_capacity(token.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.len() >= MIN_HEX_RUN {
            out.push_str(MASK);
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in token.chars() {
        if c.is_ascii_hexdigit() {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Remplace par `<uuid>` tout UUID `8-4-4-4-12`, même collé à un mot (`str/<uuid>/info`,
/// `x=<uuid>`) : seule une frontière hexadécimale l'empêche.
fn mask_uuids(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if is_uuid_at(&chars, i) {
            out.push_str("<uuid>");
            i += 36;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Vrai si un UUID `8-4-4-4-12` commence en `at` (casse indifférente).
fn is_uuid_at(chars: &[char], at: usize) -> bool {
    if at + 36 > chars.len() || (at > 0 && chars[at - 1].is_ascii_hexdigit()) {
        return false;
    }
    let ok = (0..36).all(|k| match k {
        8 | 13 | 18 | 23 => chars[at + k] == '-',
        _ => chars[at + k].is_ascii_hexdigit(),
    });
    ok && chars.get(at + 36).is_none_or(|c| !c.is_ascii_hexdigit())
}

/// Remplace `needle` par `with`, sans tenir compte de la casse (ASCII et Unicode simple).
fn replace_ci(text: &str, needle: &str, with: &str) -> String {
    replace_matching(text, needle, with, |_, _| true)
}

/// Comme [`replace_ci`], mais seulement là où `needle` n'est pas collé à une lettre ou un
/// chiffre.
fn replace_word_ci(text: &str, needle: &str, with: &str) -> String {
    replace_matching(text, needle, with, |before, after| {
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

fn replace_matching(
    text: &str,
    needle: &str,
    with: &str,
    accept: impl Fn(Option<char>, Option<char>) -> bool,
) -> String {
    let lower = |s: &str| -> Vec<char> { s.chars().flat_map(char::to_lowercase).collect() };
    let needle = lower(needle);
    if needle.is_empty() {
        return text.to_string();
    }
    // Comparaison caractère par caractère (la mise en minuscules peut changer la longueur :
    // on compare chaque caractère d'origine à sa forme minuscule d'un seul caractère).
    let chars: Vec<char> = text.chars().collect();
    let folded: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let end = i + needle.len();
        if end <= chars.len()
            && folded[i..end] == needle[..]
            && accept(i.checked_sub(1).map(|k| chars[k]), chars.get(end).copied())
        {
            out.push_str(with);
            i = end;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Remplace le segment qui suit `\Users\` ou `/Users/` (casse indifférente) par `%USERNAME%`.
fn mask_users_segment(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let sep = chars[i];
        let is_users = (sep == '\\' || sep == '/')
            && i + 7 <= chars.len()
            && chars[i + 1..i + 6]
                .iter()
                .map(|c| c.to_ascii_lowercase())
                .eq("users".chars())
            && (chars[i + 6] == '\\' || chars[i + 6] == '/');
        if !is_users {
            out.push(sep);
            i += 1;
            continue;
        }
        out.extend(&chars[i..i + 7]);
        let mut j = i + 7;
        let start = j;
        while j < chars.len() && !matches!(chars[j], '\\' | '/' | '"' | '\'' | ' ' | ')' | ':') {
            j += 1;
        }
        let segment: String = chars[start..j].iter().collect();
        if !segment.is_empty() && segment != "%USERNAME%" && segment != "Public" {
            out.push_str("%USERNAME%");
        } else {
            out.push_str(&segment);
        }
        i = j;
    }
    out
}

// --- Système ----------------------------------------------------------------------------------

/// Version de Windows (`Windows 11 (build 26100.2314, 24H2)`) et architecture.
pub fn os_description() -> String {
    format!("{}, {}", os_name(), std::env::consts::ARCH)
}

#[cfg(windows)]
fn os_name() -> String {
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
    };
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let key = wide(KEY);
    let read_sz = |name: &str| -> Option<String> {
        let name = wide(name);
        let mut buf = [0u16; 128];
        let mut len = std::mem::size_of_val(&buf) as u32;
        // SAFETY : tampon et longueur cohérents, chaînes terminées par un nul.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut len,
            )
        };
        if status != 0 {
            return None;
        }
        let units = (len as usize / 2).min(buf.len());
        let text = String::from_utf16_lossy(&buf[..units]);
        let text = text.trim_end_matches('\0').trim().to_string();
        (!text.is_empty()).then_some(text)
    };
    let read_dword = |name: &str| -> Option<u32> {
        let name = wide(name);
        let mut value = 0u32;
        let mut len = 4u32;
        // SAFETY : la valeur fait 4 octets, chaînes terminées par un nul.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&mut value as *mut u32).cast(),
                &mut len,
            )
        };
        (status == 0).then_some(value)
    };
    let build = read_sz("CurrentBuildNumber").or_else(|| read_sz("CurrentBuild"));
    let Some(build) = build else {
        return "Windows (version illisible)".into();
    };
    // Le registre dit « Windows 10 » même sous Windows 11 : le numéro de build tranche.
    let number: u32 = build.parse().unwrap_or(0);
    let name = if number >= 22000 {
        "Windows 11"
    } else {
        "Windows 10"
    };
    let ubr = read_dword("UBR")
        .map(|u| format!(".{u}"))
        .unwrap_or_default();
    let display = read_sz("DisplayVersion")
        .filter(|v| v.len() <= 16 && v.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|v| format!(", {v}"))
        .unwrap_or_default();
    format!("{name} (build {build}{ubr}{display})")
}

#[cfg(not(windows))]
fn os_name() -> String {
    std::env::consts::OS.to_string()
}

// --- Appareil ---------------------------------------------------------------------------------

/// Données d'entrée du diagnostic (injectables pour les tests).
pub struct Inputs<'a> {
    pub env: &'a Environment,
    pub ctx: &'a WriteContext<'a>,
    /// Une écriture est en cours : les contrôles du firmware ne sont pas refaits.
    pub busy: bool,
    /// UUID des histoires installées par ce launcher (compte seulement).
    pub launcher_stories: &'a HashSet<String>,
    pub os: String,
    pub errors: Vec<SessionError>,
    pub redactor: &'a Redactor,
    pub now: Instant,
}

/// Fins de ligne d'un index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineEndings {
    None,
    Lf,
    Crlf,
    Cr,
    Mixed,
}

impl LineEndings {
    fn label(self) -> &'static str {
        match self {
            Self::None => "aucune",
            Self::Lf => "LF",
            Self::Crlf => "CRLF",
            Self::Cr => "CR seul",
            Self::Mixed => "mélangées",
        }
    }
}

/// État d'un index, sans aucun UUID.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexDiag {
    state: String,
    size: Option<u64>,
    entries: usize,
    invalid_lines: usize,
    duplicates: usize,
    endings: LineEndings,
    final_newline: bool,
    bom: bool,
    non_canonical: usize,
    without_folder: usize,
}

fn line_endings(bytes: &[u8]) -> (LineEndings, bool) {
    let (mut lf, mut crlf, mut cr) = (0usize, 0usize, 0usize);
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                crlf += 1;
                i += 1;
            }
            b'\r' => cr += 1,
            b'\n' => lf += 1,
            _ => {}
        }
        i += 1;
    }
    let kinds = [lf > 0, crlf > 0, cr > 0].iter().filter(|k| **k).count();
    let endings = match (kinds, lf > 0, crlf > 0) {
        (0, _, _) => LineEndings::None,
        (1, true, _) => LineEndings::Lf,
        (1, _, true) => LineEndings::Crlf,
        (1, _, _) => LineEndings::Cr,
        _ => LineEndings::Mixed,
    };
    let final_newline = matches!(bytes.last(), Some(b'\n' | b'\r'));
    (endings, final_newline)
}

/// Lit `path` (au plus `MAX_INDEX_BYTES + 1` octets) ; `None` s'il est absent.
fn read_bounded(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
        Ok(meta) if !meta.is_file() => {
            return Err(std::io::Error::other("ce n'est pas un fichier ordinaire"))
        }
        Ok(_) => {}
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_INDEX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

fn index_diag(root: &Path, name: &str, has_folder: impl Fn(&StoryId) -> bool) -> IndexDiag {
    let path = root.join(LIBRARY_DIR).join(name);
    let mut diag = IndexDiag {
        state: "absent".into(),
        size: None,
        entries: 0,
        invalid_lines: 0,
        duplicates: 0,
        endings: LineEndings::None,
        final_newline: false,
        bom: false,
        non_canonical: 0,
        without_folder: 0,
    };
    let bytes = match read_bounded(&path) {
        Ok(None) => return diag,
        Ok(Some(bytes)) => bytes,
        Err(err) => {
            diag.state = format!("illisible ({err})");
            return diag;
        }
    };
    diag.size = Some(bytes.len() as u64);
    let read = index::parse(&bytes);
    diag.state = match &read.state {
        IndexState::Absent => "absent".into(),
        IndexState::Valid => "valide".into(),
        IndexState::Damaged => "abîmé (lignes ignorées)".into(),
        IndexState::Unreadable(reason) => format!("illisible ({reason})"),
    };
    diag.entries = read.entries.len();
    for issue in &read.issues {
        match issue {
            index::LineIssue::Invalid { .. } => diag.invalid_lines += 1,
            index::LineIssue::Duplicate { .. } => diag.duplicates += 1,
        }
    }
    (diag.endings, diag.final_newline) = line_endings(&bytes);
    diag.bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
    let text = String::from_utf8_lossy(&bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    diag.non_canonical = text
        .split(['\n', '\r'])
        .filter(|line| {
            let trimmed = line.trim();
            StoryId::parse(trimmed).is_some_and(|id| id.as_str() != *line)
        })
        .count();
    diag.without_folder = read.entries.iter().filter(|id| !has_folder(id)).count();
    diag
}

/// Dossiers de `dir` : noms au format UUID, et nombre d'autres entrées (dossiers ou fichiers).
fn scan_dir(dir: &Path) -> Option<(HashSet<StoryId>, usize)> {
    let entries = fs::read_dir(dir).ok()?;
    let mut ids = HashSet::new();
    let mut others = 0;
    for entry in entries.take(MAX_DIR_ENTRIES).flatten() {
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        match entry.file_name().to_str().and_then(StoryId::parse) {
            Some(id) if is_dir => {
                ids.insert(id);
            }
            _ => others += 1,
        }
    }
    Some((ids, others))
}

/// Nombre d'entrées d'un dossier de travail (`None` : absent).
fn work_dir(path: &Path) -> Option<Result<usize, String>> {
    match fs::symlink_metadata(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => Some(Err(err.to_string())),
        Ok(meta) if !meta.is_dir() => Some(Err("ce n'est pas un dossier".into())),
        Ok(_) => Some(
            fs::read_dir(path)
                .map(|it| it.take(MAX_DIR_ENTRIES).count())
                .map_err(|e| e.to_string()),
        ),
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "oui"
    } else {
        "non"
    }
}

fn bytes_text(n: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let n_f = n as f64;
    let human = if n_f >= GIB {
        format!("{:.2} Gio", n_f / GIB)
    } else if n_f >= MIB {
        format!("{:.1} Mio", n_f / MIB)
    } else {
        format!("{:.1} Kio", n_f / 1024.0)
    };
    format!("{} ({n} octets)", human.replace('.', ","))
}

fn ago(now: Instant, at: Instant) -> String {
    let secs = now.saturating_duration_since(at).as_secs();
    match secs {
        0..=59 => format!("il y a {secs} s"),
        60..=3599 => format!("il y a {} min", secs / 60),
        _ => format!("il y a {} h", secs / 3600),
    }
}

fn family_text(family: Family) -> &'static str {
    match family {
        Family::V1 => "1.x",
        Family::V2 => "2.x",
        Family::Unsupported => "non prise en charge",
        Family::Unreadable => "illisible",
    }
}

fn form_text(form: VersionForm) -> &'static str {
    match form {
        VersionForm::Main => "préfixe « main: »",
        VersionForm::Bare => "version nue",
        VersionForm::Unreadable => "illisible",
    }
}

fn trust_text(trust: Trust) -> &'static str {
    match trust {
        Trust::NotApplicable => "sans objet (not_applicable)",
        Trust::FlamVerified => "vérifié par la Flam (flam_verified)",
        Trust::Confirmed => "sauvegarde confirmée (confirmed)",
        Trust::Unconfirmed => "confirmation à donner (unconfirmed)",
    }
}

fn coherence_text(coherence: &Coherence) -> String {
    let (label, counts) = match coherence {
        Coherence::NotApplicable => return "sans objet (not_applicable)".into(),
        Coherence::NotChecked => return "non vérifiée (not_checked)".into(),
        Coherence::Coherent { counts } => ("cohérente (coherent)", counts),
        // Les UUID en cause ne sont jamais repris (des nombres seulement).
        Coherence::Contradiction { counts, .. } => ("contradiction", counts),
        Coherence::NoEvidence { counts } => ("aucune preuve (no_evidence)", counts),
    };
    format!(
        "{label} : examinées {}, fichier key attendu {}, déchiffrées {}, déchiffrées avec un autre key {}, format Lunii {}, info en clair {}",
        counts.examined,
        counts.key_matches,
        counts.decrypted,
        counts.decrypted_other_key,
        counts.lunii_format,
        counts.clear_info
    )
}

fn write_block_text(block: &Option<guard::WriteRefused>) -> String {
    match block {
        None => "autorisée".into(),
        Some(reason) => {
            let kind = serde_json::to_value(reason)
                .ok()
                .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_string))
                .unwrap_or_default();
            format!("refusée ({kind}) : {reason}")
        }
    }
}

fn firmware_section(out: &mut String, status: &FirmwareStatus) {
    let _ = writeln!(out, "### Firmware\n");
    let _ = writeln!(
        out,
        "- Version : {} (famille {}, {})",
        status.version.as_deref().unwrap_or("illisible"),
        family_text(status.family),
        form_text(status.version_form)
    );
    let _ = writeln!(
        out,
        "- .mdf : {} octets, en-tête {}, disposition standard : {}",
        status.mdf_size,
        status.header.as_deref().unwrap_or("illisible"),
        yes_no(status.layout_standard)
    );
    let _ = writeln!(
        out,
        "- VID:PID : {}",
        status.usb_ids.as_deref().unwrap_or("illisible")
    );
    let _ = writeln!(
        out,
        "- Cohérence des clés : {}",
        coherence_text(&status.coherence)
    );
    let _ = writeln!(out, "- Confiance : {}", trust_text(status.trust));
    let _ = writeln!(
        out,
        "- Écriture : {}\n",
        write_block_text(&status.write_block)
    );
}

fn index_table(out: &mut String, visible: &IndexDiag, hidden: &IndexDiag) {
    let _ = writeln!(out, "| Index | list | list.hidden |");
    let _ = writeln!(out, "|---|---|---|");
    let row = |out: &mut String, label: &str, f: &dyn Fn(&IndexDiag) -> String| {
        let _ = writeln!(out, "| {label} | {} | {} |", f(visible), f(hidden));
    };
    row(out, "État", &|d| d.state.replace('|', "/"));
    row(out, "Taille (octets)", &|d| {
        d.size.map_or_else(|| "—".into(), |s| s.to_string())
    });
    row(out, "Entrées", &|d| d.entries.to_string());
    row(out, "Lignes invalides", &|d| d.invalid_lines.to_string());
    row(out, "Doublons", &|d| d.duplicates.to_string());
    row(out, "Fins de ligne", &|d| d.endings.label().to_string());
    row(out, "Saut de ligne final", &|d| {
        yes_no(d.final_newline).into()
    });
    row(out, "BOM UTF-8", &|d| yes_no(d.bom).into());
    row(out, "UUID non canoniques", &|d| d.non_canonical.to_string());
    row(out, "Entrées sans dossier", &|d| {
        d.without_folder.to_string()
    });
    out.push('\n');
}

fn device_section(out: &mut String, inputs: &Inputs<'_>, device: &detect::DetectedDevice) {
    let root = &device.root;
    let _ = writeln!(out, "### Flam\n");
    let _ = writeln!(
        out,
        "- Système de fichiers : {}",
        if device.file_system.is_empty() {
            "illisible"
        } else {
            &device.file_system
        }
    );
    match device.space {
        Some(Space { free, total }) => {
            let _ = writeln!(
                out,
                "- Espace : {} libres sur {}",
                bytes_text(free),
                bytes_text(total)
            );
        }
        None => {
            let _ = writeln!(out, "- Espace : indisponible");
        }
    }
    let warnings: Vec<String> = device
        .warnings
        .iter()
        .map(|w| match w {
            detect::DeviceWarning::MissingStrDir => "dossier str/ absent".to_string(),
            detect::DeviceWarning::MissingLibraryDir => "dossier etc/library/ absent".to_string(),
            detect::DeviceWarning::UnexpectedMdfSize { size } => {
                format!(".mdf de taille inattendue ({size} octets)")
            }
        })
        .chain(device.mdf_error.iter().filter_map(|e| match e {
            MdfError::FormatUnspecified => None,
            other => Some(other.to_string()),
        }))
        .collect();
    let _ = writeln!(
        out,
        "- Avertissements : {}\n",
        if warnings.is_empty() {
            "aucun".to_string()
        } else {
            warnings.join(" ; ")
        }
    );

    let status = match device.mdf_bytes.as_deref() {
        Some(mdf) => guard::firmware_status(root, mdf, inputs.ctx, inputs.busy),
        None => guard::unreadable_status(
            device.mdf_size,
            device
                .mdf_error
                .as_ref()
                .map_or_else(|| "illisible".to_string(), ToString::to_string),
        ),
    };
    firmware_section(out, &status);

    // Index et dossiers d'histoires.
    let (str_ids, str_others) = scan_dir(&root.join(index::STORIES_DIR)).unwrap_or_default();
    let str_readable = root.join(index::STORIES_DIR).is_dir();
    let (hidden_ids, hidden_others) = scan_dir(&root.join(HIDDEN_STORIES_DIR)).unwrap_or_default();
    let visible = index_diag(root, "list", |id| str_ids.contains(id));
    let hidden = index_diag(root, "list.hidden", |id| {
        str_ids.contains(id) || hidden_ids.contains(id)
    });
    let _ = writeln!(out, "### Index\n");
    index_table(out, &visible, &hidden);
    let listed: HashSet<StoryId> = [
        index::read_index(root, index::IndexKind::Visible).entries,
        index::read_index(root, index::IndexKind::Hidden).entries,
    ]
    .concat()
    .into_iter()
    .collect();
    let out_of_index =
        |ids: &HashSet<StoryId>| ids.iter().filter(|id| !listed.contains(id)).count();
    let _ = writeln!(
        out,
        "- Dossiers hors index : str/ {}, str.hidden/ {}",
        out_of_index(&str_ids),
        out_of_index(&hidden_ids)
    );
    let _ = writeln!(
        out,
        "- Autres entrées (nom non UUID) : str/ {}, str.hidden/ {}{}",
        str_others,
        hidden_others,
        if str_readable {
            ""
        } else {
            " (str/ absent ou illisible)"
        }
    );
    let all: HashSet<&StoryId> = listed.iter().chain(&str_ids).chain(&hidden_ids).collect();
    let mine = all
        .iter()
        .filter(|id| inputs.launcher_stories.contains(id.as_str()))
        .count();
    let _ = writeln!(
        out,
        "- Histoires : {} (installées par ce launcher : {mine} ; autres, officielles ou d'autres outils : {})\n",
        all.len(),
        all.len() - mine
    );

    // Fichiers de travail.
    let _ = writeln!(out, "### Fichiers de travail\n");
    for name in [TRANSFER_DIR, TRASH_DIR] {
        let text = match work_dir(&root.join(name)) {
            None => "absent".to_string(),
            Some(Ok(n)) => format!("présent ({n} entrée(s))"),
            Some(Err(err)) => format!("illisible ({err})"),
        };
        let _ = writeln!(out, "- {name} : {text}");
    }
    let mut leftovers = Vec::new();
    if let Ok(entries) = fs::read_dir(root.join(LIBRARY_DIR)) {
        for entry in entries.take(MAX_DIR_ENTRIES).flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let work = ["list", "list.hidden"].iter().any(|base| {
                name.strip_prefix(base).is_some_and(|rest| {
                    matches!(rest, ".new" | ".bak" | ".restore") || rest.starts_with(".corrupt")
                })
            });
            if work {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                leftovers.push(format!("{name} ({size} octets)"));
            }
        }
    }
    leftovers.sort();
    let _ = writeln!(
        out,
        "- etc/library : {}",
        if leftovers.is_empty() {
            "aucun fichier de travail (list.new, list.bak, list.restore, list.corrupt*)".to_string()
        } else {
            leftovers.join(", ")
        }
    );
    let cache = match fs::symlink_metadata(root.join(LIBRARY_CACHE)) {
        Ok(meta) if meta.is_file() => format!("présent, {} octets", meta.len()),
        Ok(_) => "présent, pas un fichier".to_string(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => "absent".to_string(),
        Err(err) => format!("illisible ({err})"),
    };
    let _ = writeln!(out, "- {LIBRARY_CACHE} : {cache}\n");
}

/// Texte Markdown du diagnostic (voir le module). Lecture seule.
pub fn build(inputs: &Inputs<'_>) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "## Diagnostic flam-launcher\n");
    let _ = writeln!(out, "- Launcher : {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(out, "- Système : {}", inputs.os);

    let report = detect::detect(inputs.env);
    let _ = writeln!(out, "- Flam détectées : {}\n", report.devices.len());
    match report.devices.first() {
        Some(device) => device_section(&mut out, inputs, device),
        None => {
            let _ = writeln!(out, "Aucune Flam détectée : diagnostic réduit.\n");
        }
    }

    let _ = writeln!(out, "### Erreurs de la session\n");
    if inputs.errors.is_empty() {
        let _ = writeln!(out, "- aucune");
    } else {
        for error in inputs.errors.iter().rev() {
            let _ = writeln!(
                out,
                "- {} ({}) : {}",
                error.operation,
                ago(inputs.now, error.at),
                inputs.redactor.clean_message(&error.message)
            );
        }
    }
    inputs.redactor.scrub(&out)
}

/// Diagnostic à coller dans une issue « Retour testeur Flam ». Lecture seule : rien n'est écrit
/// sur la Flam, aucun accès réseau. Disponible sans Flam (diagnostic réduit) et quand la garde
/// refuse l'écriture.
#[tauri::command]
pub async fn device_diagnostic(config: State<'_, ConfigState>) -> AppResult<String> {
    let cache_root = super::transfer::launcher_cache_root(&config);
    blocking(move || {
        let env = Environment::current();
        let ctx = WriteContext::current(cache_root.clone());
        let launcher = super::status::launcher_story_ids(cache_root.as_deref());
        let redactor = Redactor::current();
        build(&Inputs {
            env: &env,
            ctx: &ctx,
            busy: DEVICE_LOCK.current().is_some(),
            launcher_stories: &launcher,
            os: os_description(),
            errors: session_errors(),
            redactor: &redactor,
            now: Instant::now(),
        })
    })
    .await
}

#[cfg(test)]
mod tests;
