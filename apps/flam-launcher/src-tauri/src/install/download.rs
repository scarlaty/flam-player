//! Téléchargement en flux d'une archive : fichier `.part`, reprise (`Range` + `If-Range`),
//! vérification de la taille, annulation, puis renommage.

use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::header::{CONTENT_RANGE, ETAG, IF_RANGE, LAST_MODIFIED, RANGE};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

use crate::catalog::{redirect_policy, USER_AGENT};
use crate::{AppError, AppResult};

/// Délai de connexion, et délai maximal sans recevoir d'octet.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// Validateur HTTP mémorisé pour reprendre un téléchargement (`If-Range`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Validator {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl Validator {
    /// Valeur d'`If-Range` : un ETag fort, sinon la date de modification. Un ETag faible
    /// (`W/`) ne permet pas une reprise sûre.
    fn if_range(&self) -> Option<&str> {
        match &self.etag {
            Some(etag) if !etag.starts_with("W/") => Some(etag),
            _ => self.last_modified.as_deref(),
        }
    }
}

/// Client HTTP des téléchargements : comme celui du catalogue (rustls, redirections bornées,
/// jamais https vers http, `User-Agent`), mais sans délai total (archives de plusieurs
/// centaines de Mio) : un délai de lecture borne seulement les silences.
pub fn build_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .use_rustls_tls()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .redirect(reqwest::redirect::Policy::custom(redirect_policy))
        .user_agent(USER_AGENT)
        .build()
}

/// Paramètres d'un téléchargement.
pub struct Download<'a> {
    pub client: &'a reqwest::Client,
    pub url: &'a str,
    /// Fichier final (publié par renommage du `.part` une fois complet et vérifié).
    pub dest: &'a Path,
    /// Taille maximale acceptée.
    pub max_bytes: u64,
}

/// `<dest>.part`.
pub fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

fn network(err: &reqwest::Error) -> AppError {
    let reason = if err.is_timeout() {
        "délai dépassé"
    } else if err.is_redirect() {
        "redirection refusée ou trop longue"
    } else if err.is_connect() {
        "connexion impossible"
    } else {
        "échec de la requête"
    };
    AppError::Network(format!("téléchargement impossible : {reason}"))
}

/// `Content-Range: bytes <début>-<fin>/<total>` -> (début, total si connu).
fn parse_content_range(value: &str) -> Option<(u64, Option<u64>)> {
    let rest = value.trim().strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let start = range.split_once('-')?.0.trim().parse().ok()?;
    let total = total.trim().parse().ok();
    Some((start, total))
}

fn header(response: &reqwest::Response, name: reqwest::header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// Résultat d'une tentative de requête.
enum Attempt {
    Done,
    /// Le serveur refuse la reprise (416) : recommencer de zéro.
    Restart,
}

/// Télécharge `url` vers `dest` en reprenant `<dest>.part` s'il existe et qu'un validateur
/// est connu. `validator` est mis à jour (et `on_validator` appelé pour le persister) dès la
/// réponse reçue. `on_progress(reçus, total)` est appelé à chaque morceau. Annulation via
/// `cancel` : le `.part` est **conservé** (l'appelant décide de le supprimer).
pub async fn download(
    req: &Download<'_>,
    validator: &mut Validator,
    cancel: &mut watch::Receiver<bool>,
    on_validator: &mut (dyn FnMut(&Validator) + Send),
    on_progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
) -> AppResult<()> {
    let part = part_path(req.dest);
    for _ in 0..2 {
        match attempt(req, &part, validator, cancel, on_validator, on_progress).await? {
            Attempt::Done => {
                tokio::fs::rename(&part, req.dest).await?;
                return Ok(());
            }
            Attempt::Restart => {
                let _ = tokio::fs::remove_file(&part).await;
                *validator = Validator::default();
            }
        }
    }
    Err(AppError::DownloadInvalid(
        "le serveur refuse la reprise du téléchargement".into(),
    ))
}

async fn attempt(
    req: &Download<'_>,
    part: &Path,
    validator: &mut Validator,
    cancel: &mut watch::Receiver<bool>,
    on_validator: &mut (dyn FnMut(&Validator) + Send),
    on_progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
) -> AppResult<Attempt> {
    let existing = tokio::fs::metadata(part).await.map_or(0, |m| m.len());
    // Reprise seulement avec un validateur : sans lui, rien ne garantit que les octets déjà
    // reçus appartiennent au même fichier.
    let resume_from = match validator.if_range() {
        Some(_) if existing > 0 => existing,
        _ => 0,
    };
    let mut request = req.client.get(req.url);
    if resume_from > 0 {
        request = request
            .header(RANGE, format!("bytes={resume_from}-"))
            .header(IF_RANGE, validator.if_range().unwrap_or_default());
    }
    let response = tokio::select! {
        r = request.send() => r.map_err(|e| network(&e))?,
        _ = cancelled(cancel) => return Err(AppError::Cancelled),
    };
    let status = response.status();
    if status == StatusCode::RANGE_NOT_SATISFIABLE {
        return Ok(Attempt::Restart);
    }
    if !status.is_success() {
        return Err(AppError::Network(format!(
            "téléchargement impossible : réponse HTTP {}",
            status.as_u16()
        )));
    }
    let (mut received, total) = if status == StatusCode::PARTIAL_CONTENT && resume_from > 0 {
        match header(&response, CONTENT_RANGE).and_then(|v| parse_content_range(&v)) {
            Some((start, total)) if start == resume_from => (resume_from, total),
            _ => return Ok(Attempt::Restart),
        }
    } else {
        (0, response.content_length())
    };
    if total.is_some_and(|t| t > req.max_bytes) {
        return Err(AppError::DownloadInvalid(format!(
            "archive trop volumineuse ({} octets, {} au plus)",
            total.unwrap_or_default(),
            req.max_bytes
        )));
    }
    *validator = Validator {
        etag: header(&response, ETAG),
        last_modified: header(&response, LAST_MODIFIED),
    };
    on_validator(validator);

    let mut file = if received > 0 {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(part)
            .await?
    } else {
        tokio::fs::File::create(part).await?
    };
    on_progress(received, total);
    let mut response = response;
    loop {
        let chunk = tokio::select! {
            c = response.chunk() => c.map_err(|e| network(&e))?,
            _ = cancelled(cancel) => {
                file.flush().await?;
                return Err(AppError::Cancelled);
            }
        };
        let Some(chunk) = chunk else { break };
        received += chunk.len() as u64;
        if received > req.max_bytes || total.is_some_and(|t| received > t) {
            drop(file);
            let _ = tokio::fs::remove_file(part).await;
            return Err(AppError::DownloadInvalid(
                "archive plus volumineuse que la taille annoncée ou permise".into(),
            ));
        }
        file.write_all(&chunk).await?;
        on_progress(received, total);
    }
    file.flush().await?;
    file.sync_all().await?;
    drop(file);
    if let Some(total) = total {
        if received != total {
            // Connexion coupée : le `.part` est gardé pour une reprise.
            return Err(AppError::DownloadInvalid(format!(
                "téléchargement incomplet : {received} octets reçus sur {total}"
            )));
        }
    }
    Ok(Attempt::Done)
}

/// Se termine quand l'annulation est demandée (ou que l'émetteur disparaît).
pub async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    while !*cancel.borrow_and_update() {
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range() {
        assert_eq!(
            parse_content_range("bytes 10-99/100"),
            Some((10, Some(100)))
        );
        assert_eq!(parse_content_range("bytes 10-99/*"), Some((10, None)));
        assert_eq!(parse_content_range("octets 1-2/3"), None);
    }

    #[test]
    fn validateur_if_range() {
        let v = |etag: Option<&str>, lm: Option<&str>| Validator {
            etag: etag.map(Into::into),
            last_modified: lm.map(Into::into),
        };
        assert_eq!(v(Some("\"a\""), Some("d")).if_range(), Some("\"a\""));
        assert_eq!(v(Some("W/\"a\""), Some("d")).if_range(), Some("d"));
        assert_eq!(v(Some("W/\"a\""), None).if_range(), None);
        assert_eq!(v(None, None).if_range(), None);
    }

    #[test]
    fn chemin_part() {
        assert_eq!(
            part_path(Path::new("/c/archives/a-1.zip")),
            PathBuf::from("/c/archives/a-1.zip.part")
        );
    }
}
