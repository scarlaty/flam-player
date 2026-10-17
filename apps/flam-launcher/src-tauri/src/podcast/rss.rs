//! Lecture tolérante d'un flux RSS 2.0 avec extensions iTunes (#210).
//!
//! Logique pure : le contenu du flux est déjà téléchargé, aucun accès réseau ni Tauri. La
//! conversion en `StoryEntry` (identifiant stable dérivé de l'URL du flux, genre d'archive) est
//! laissée à la source « Podcast » (#211) : elle dépend de l'URL du flux, que ce module ignore.
//!
//! # Garanties
//!
//! - **Bornes** : flux de 16 Mio au plus, profondeur d'éléments limitée, 5 000 épisodes au
//!   plus, longueur de chaque chaîne bornée, texte accumulé par élément borné.
//! - **Sûreté XML** : toute déclaration `<!DOCTYPE …>` est refusée. Aucune entité n'est donc
//!   jamais définie (pas de « billion laughs »), aucune DTD ni entité externe n'est lue. Seules
//!   les cinq entités prédéfinies XML, les références numériques et `&nbsp;` sont résolues ;
//!   toute autre entité est ignorée.
//! - **Encodages** : UTF-8 (avec ou sans BOM), UTF-16 avec BOM, ISO-8859-1 et Windows-1252
//!   (selon la déclaration XML). Tout autre encodage déclaré est refusé.
//! - **Tolérance** : champs absents, ordre quelconque des balises, dates RFC 822 (ou ISO 8601),
//!   `<![CDATA[…]]>`, HTML dans les descriptions (retiré), balises inconnues ignorées.
//! - Un épisode sans enclosure audio (type MIME `audio/…`) valide, de type bande-annonce
//!   (`itunes:episodeType` = `trailer`) ou en doublon de `guid` est ignoré avec un
//!   avertissement.
//!
//! # Repli documenté
//!
//! - Date de publication absente ou illisible : `pub_date` vaut `None` (jamais 1970).
//! - Image du canal : balise `image`, sinon `itunes:image` du canal, sinon image du premier
//!   épisode conservé ; sinon `None` (pas de rejet : les pochettes sont traitées par #215).
//! - Image d'un épisode : `itunes:image` de l'épisode, sinon image du canal.
//! - Auteur d'un épisode : `itunes:author`, sinon `author` ou `dc:creator`, sinon auteur du
//!   canal (`itunes:author`), sinon `copyright`.
//! - Identifiant `guid` absent : URL de l'enclosure.
//! - Description : `description`, sinon `itunes:summary`.

use std::borrow::Cow;
use std::collections::HashSet;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use thiserror::Error;
use url::Url;

/// Taille maximale d'un flux (comme les catalogues).
pub const MAX_FEED_BYTES: usize = 16 * 1024 * 1024;
/// Nombre maximal d'épisodes conservés (les suivants sont ignorés).
pub const MAX_EPISODES: usize = 5_000;
/// Profondeur maximale d'imbrication des éléments.
const MAX_DEPTH: usize = 32;
/// Nombre maximal d'avertissements détaillés conservés (le reste est résumé).
const MAX_WARNINGS: usize = 50;
/// Texte accumulé au plus par élément, en octets (avant nettoyage et troncature).
const MAX_TEXT_BYTES: usize = 256 * 1024;
/// Longueurs maximales, en caractères.
const MAX_TITLE_CHARS: usize = 300;
const MAX_DESCRIPTION_CHARS: usize = 4_000;
const MAX_AUTHOR_CHARS: usize = 200;
const MAX_GUID_CHARS: usize = 512;
const MAX_MIME_CHARS: usize = 100;
/// Longueur maximale d'une URL (en octets) ; au-delà, elle est ignorée.
const MAX_URL_BYTES: usize = 2_048;

/// Erreurs de lecture d'un flux, chacune avec un message distinct.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RssError {
    /// Flux trop grand.
    #[error("flux trop volumineux : {size} octets (maximum {max})")]
    FeedTooLarge { size: usize, max: usize },
    /// Flux contenant une déclaration `DOCTYPE` (DTD, entités) : refusé par sécurité.
    #[error("flux refusé : les déclarations DOCTYPE (DTD, entités) ne sont pas acceptées")]
    DtdRefused,
    /// Encodage de caractères non pris en charge.
    #[error("encodage de caractères non pris en charge : {0}")]
    UnsupportedEncoding(String),
    /// XML mal formé, trop profond, ou qui n'est pas un flux RSS 2.0.
    #[error("flux invalide : {0}")]
    InvalidFeed(String),
    /// Flux valide mais sans aucun épisode audio exploitable.
    #[error("aucun épisode audio exploitable dans le flux")]
    NoAudioEpisode,
}

/// Un épisode exploitable (enclosure audio valide).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodcastEpisode {
    /// Identifiant de l'épisode : `guid`, sinon URL de l'enclosure.
    pub guid: String,
    pub title: String,
    /// Description, HTML retiré.
    pub description: Option<String>,
    /// Date de publication, ISO 8601 UTC (`2008-06-03T11:05:30Z`) ; `None` si absente ou
    /// illisible.
    pub pub_date: Option<String>,
    /// Durée en secondes (`itunes:duration`), si connue.
    pub duration: Option<u32>,
    pub author: Option<String>,
    /// URL (http ou https) de l'image de l'épisode, sinon celle du canal.
    pub image: Option<String>,
    /// URL (http ou https) de l'enclosure audio.
    pub enclosure_url: String,
    /// Type MIME de l'enclosure (commence par `audio/`).
    pub enclosure_type: String,
    /// Taille annoncée de l'enclosure en octets, si connue (non vérifiée).
    pub enclosure_length: Option<u64>,
}

/// Un flux podcast normalisé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodcastFeed {
    pub title: String,
    /// Description, HTML retiré.
    pub description: Option<String>,
    /// URL (http ou https) de la pochette du canal.
    pub image: Option<String>,
    /// URL (http ou https) du site.
    pub link: Option<String>,
    pub copyright: Option<String>,
    /// Épisodes exploitables, dans l'ordre du flux.
    pub episodes: Vec<PodcastEpisode>,
    /// Épisodes ou éléments ignorés, au plus 50 messages détaillés.
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------------------
// Texte
// ---------------------------------------------------------------------------------------

/// Tronque à `max` caractères.
fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((end, _)) => s[..end].to_string(),
        None => s.to_string(),
    }
}

/// Texte sur une ligne : caractères de contrôle remplacés par une espace, espaces réduites.
fn clean_line(s: &str, max: usize) -> String {
    let joined = s
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    truncate_chars(&joined, max)
}

fn non_empty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

/// Entité HTML nommée courante (hors `amp`, `lt`, `gt`, `quot`, `apos`, `nbsp`).
fn html_named_entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "hellip" => '…',
        "mdash" => '—',
        "ndash" => '–',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "laquo" => '«',
        "raquo" => '»',
        "euro" => '€',
        "copy" => '©',
        "eacute" => 'é',
        "egrave" => 'è',
        "ecirc" => 'ê',
        "agrave" => 'à',
        "acirc" => 'â',
        "ccedil" => 'ç',
        "ocirc" => 'ô',
        "ucirc" => 'û',
        "ugrave" => 'ù',
        "icirc" => 'î',
        "euml" => 'ë',
        _ => return None,
    })
}

/// Décode une référence numérique `#123` ou `#x1F` (sans le `&` ni le `;`).
fn numeric_ref(body: &str) -> Option<char> {
    let digits = body.strip_prefix('#')?;
    let code = match digits.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => digits.parse::<u32>().ok()?,
    };
    char::from_u32(code).filter(|c| *c != '\0')
}

/// Décode une seule fois les entités HTML d'un texte déjà débarrassé de ses balises.
fn decode_html_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        let decoded = after.find(';').filter(|end| *end <= 10).and_then(|end| {
            let body = &after[..end];
            let ch = if body.starts_with('#') {
                numeric_ref(body)
            } else {
                html_named_entity(body)
            };
            ch.map(|c| (c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Retire les balises HTML d'une description (contenu de `script` et `style` compris) ; les
/// blocs (`p`, `br`, `div`, `li`, titres…) deviennent des sauts de ligne. Les entités sont
/// décodées après, une seule fois : `&lt;b&gt;` donne le texte `<b>`, jamais une balise.
pub fn strip_html(input: &str) -> String {
    let mut text = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(pos) = rest.find('<') {
        text.push_str(&rest[..pos]);
        let tag_area = &rest[pos + 1..];
        let first = tag_area.chars().next();
        if !matches!(first, Some(c) if c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?')) {
            // Un « < » isolé est du texte.
            text.push('<');
            rest = tag_area;
            continue;
        }
        if let Some(comment) = tag_area.strip_prefix("!--") {
            rest = comment.find("-->").map_or("", |end| &comment[end + 3..]);
            continue;
        }
        let Some(end) = tag_area.find('>') else {
            // Balise jamais fermée : le reste est abandonné.
            rest = "";
            break;
        };
        let tag = &tag_area[..end];
        rest = &tag_area[end + 1..];
        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if !closing && (name == "script" || name == "style") {
            let close = format!("</{name}");
            let lower = rest.to_ascii_lowercase();
            rest = match lower.find(&close) {
                Some(i) => rest[i..].find('>').map_or("", |e| &rest[i + e + 1..]),
                None => "",
            };
            continue;
        }
        if matches!(
            name.as_str(),
            "br" | "p"
                | "div"
                | "li"
                | "ul"
                | "ol"
                | "tr"
                | "blockquote"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
        ) {
            text.push('\n');
        }
    }
    text.push_str(rest);
    normalize_block_text(&decode_html_entities(&text))
}

/// Espaces réduites sur chaque ligne, lignes vides consécutives réduites à une.
fn normalize_block_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank = 0usize;
    for line in s.lines() {
        let line = line
            .split(|c: char| c.is_whitespace() || c.is_control())
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if line.is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
            if blank > 0 {
                out.push('\n');
            }
        }
        blank = 0;
        out.push_str(&line);
    }
    out
}

fn description_text(raw: &str) -> Option<String> {
    non_empty(truncate_chars(&strip_html(raw), MAX_DESCRIPTION_CHARS))
}

/// URL http(s) valide et de longueur raisonnable, sinon `None`.
fn http_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > MAX_URL_BYTES {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    let web = matches!(url.scheme(), "http" | "https") && url.host_str().is_some();
    web.then(|| url.to_string())
}

// ---------------------------------------------------------------------------------------
// Dates et durées
// ---------------------------------------------------------------------------------------

/// Jours depuis 1970-01-01 pour une date du calendrier grégorien (algorithme de H. Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

/// Construit l'horodatage ISO 8601 UTC ; `offset_minutes` est le décalage du fuseau d'origine.
fn iso_utc(
    y: i64,
    mo: i64,
    d: i64,
    h: i64,
    mi: i64,
    s: i64,
    offset_minutes: i64,
) -> Option<String> {
    if !(1900..=9999).contains(&y)
        || !(1..=12).contains(&mo)
        || d < 1
        || d > days_in_month(y, mo)
        || !(0..24).contains(&h)
        || !(0..60).contains(&mi)
        || !(0..=60).contains(&s)
    {
        return None;
    }
    let secs =
        days_from_civil(y, mo, d) * 86_400 + h * 3_600 + mi * 60 + s.min(59) - offset_minutes * 60;
    let (y, mo, d) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    if !(1900..=9999).contains(&y) {
        return None;
    }
    Some(format!(
        "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    ))
}

fn month_number(token: &str) -> Option<i64> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let lower = token.get(..3)?.to_ascii_lowercase();
    MONTHS
        .iter()
        .position(|m| *m == lower)
        .map(|i| i as i64 + 1)
}

/// `HH:MM[:SS]` (fraction de seconde ignorée).
fn parse_clock(token: &str) -> Option<(i64, i64, i64)> {
    let mut parts = token.split(':');
    let h = parts.next()?.parse().ok()?;
    let m = parts.next()?.parse().ok()?;
    let s = match parts.next() {
        Some(sec) => sec.split('.').next()?.parse().ok()?,
        None => 0,
    };
    parts.next().is_none().then_some((h, m, s))
}

/// Décalage en minutes d'un fuseau RFC 822 ou numérique (`+0200`, `-05:00`) ; inconnu : 0.
fn zone_offset(token: &str) -> i64 {
    match token.to_ascii_uppercase().as_str() {
        "EST" | "CDT" => return -300,
        "EDT" => return -240,
        "CST" | "MDT" => return -360,
        "MST" | "PDT" => return -420,
        "PST" => return -480,
        _ => {}
    }
    let Some(sign) = token
        .strip_prefix('+')
        .map(|_| 1)
        .or_else(|| token.strip_prefix('-').map(|_| -1))
    else {
        return 0;
    };
    let digits: String = token[1..].chars().filter(char::is_ascii_digit).collect();
    if digits.len() != 4 || token[1..].chars().any(|c| !c.is_ascii_digit() && c != ':') {
        return 0;
    }
    let (h, m): (i64, i64) = (
        digits[..2].parse().unwrap_or(0),
        digits[2..].parse().unwrap_or(0),
    );
    if h > 23 || m > 59 {
        return 0;
    }
    sign * (h * 60 + m)
}

/// Lit une date RFC 822 / 2822 (`Tue, 03 Jun 2008 11:05:30 GMT`, jour de semaine, secondes et
/// fuseau facultatifs, année à deux chiffres) ou, à défaut, ISO 8601, et la rend en ISO 8601
/// UTC. `None` si illisible.
pub fn parse_pub_date(raw: &str) -> Option<String> {
    let cleaned = raw.replace(',', " ");
    let mut tokens: Vec<&str> = cleaned.split_whitespace().collect();
    if tokens.len() > 8 {
        return None;
    }
    if tokens
        .first()
        .is_some_and(|t| t.chars().all(|c| c.is_ascii_alphabetic()) && month_number(t).is_none())
    {
        tokens.remove(0);
    }
    if let [day, month, year, time, rest @ ..] = tokens.as_slice() {
        if let (Ok(d), Some(mo), Ok(y), Some((h, mi, s))) = (
            day.parse::<i64>(),
            month_number(month),
            year.parse::<i64>(),
            parse_clock(time),
        ) {
            let y = match year.len() {
                1 | 2 if y < 50 => 2000 + y,
                1 | 2 => 1900 + y,
                3 => 1900 + y,
                _ => y,
            };
            let offset = rest.first().map_or(0, |z| zone_offset(z));
            return iso_utc(y, mo, d, h, mi, s, offset);
        }
    }
    parse_iso_date(raw.trim())
}

fn parse_iso_date(raw: &str) -> Option<String> {
    let (date, time) = match raw.split_once(['T', 't', ' ']) {
        Some((d, t)) => (d, Some(t.trim())),
        None => (raw, None),
    };
    let mut dp = date.split('-');
    let y: i64 = dp.next()?.parse().ok()?;
    let mo: i64 = dp.next()?.parse().ok()?;
    let d: i64 = dp.next()?.parse().ok()?;
    if dp.next().is_some() {
        return None;
    }
    let (h, mi, s, offset) = match time {
        None => (0, 0, 0, 0),
        Some(t) => {
            let zone_at = t.find(['Z', 'z', '+', '-']);
            let (clock, zone) = match zone_at {
                Some(i) => (&t[..i], &t[i..]),
                None => (t, ""),
            };
            let (h, mi, s) = parse_clock(clock)?;
            let offset = if zone.eq_ignore_ascii_case("z") {
                0
            } else {
                zone_offset(zone)
            };
            (h, mi, s, offset)
        }
    };
    iso_utc(y, mo, d, h, mi, s, offset)
}

/// `itunes:duration` : secondes (`3600`), `MM:SS` ou `HH:MM:SS`. Non nul, plafonné à 100 h.
fn parse_duration(raw: &str) -> Option<u32> {
    let raw = raw.trim();
    let parts: Vec<&str> = raw.split(':').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let mut total: u64 = 0;
    for part in &parts {
        // Une fraction de seconde (`12.5`) est tronquée.
        let n: u64 = part.split('.').next()?.trim().parse().ok()?;
        total = total.checked_mul(60)?.checked_add(n)?;
    }
    (total > 0 && total <= 100 * 3_600).then_some(total as u32)
}

// ---------------------------------------------------------------------------------------
// Encodage
// ---------------------------------------------------------------------------------------

/// Windows-1252 : 0x80-0x9F, les cinq octets non définis restent des caractères de contrôle.
const CP1252_HIGH: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}',
    '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
];

/// Encodage déclaré dans le prologue XML (`<?xml … encoding="…"?>`), en minuscules.
fn declared_encoding(bytes: &[u8]) -> Option<String> {
    let head = &bytes[..bytes.len().min(200)];
    if !head.starts_with(b"<?xml") {
        return None;
    }
    let end = head
        .windows(2)
        .position(|w| w == b"?>")
        .unwrap_or(head.len());
    let decl = String::from_utf8_lossy(&head[..end]).to_ascii_lowercase();
    let after = decl.split("encoding").nth(1)?;
    let after = after.trim_start().strip_prefix('=')?.trim_start();
    let quote = after.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let value = after[1..].split(quote).next()?;
    Some(value.trim().to_string())
}

fn utf16_to_string(bytes: &[u8], big_endian: bool) -> Result<String, RssError> {
    // Un éventuel octet final isolé est ignoré.
    let units = bytes.as_chunks::<2>().0.iter().map(|&pair| {
        if big_endian {
            u16::from_be_bytes(pair)
        } else {
            u16::from_le_bytes(pair)
        }
    });
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|_| RssError::InvalidFeed("UTF-16 invalide".into()))
}

/// Convertit le flux en UTF-8 (voir les encodages pris en charge en tête de module).
fn decode_to_utf8(bytes: &[u8]) -> Result<Cow<'_, str>, RssError> {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16_to_string(rest, false).map(Cow::Owned);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16_to_string(rest, true).map(Cow::Owned);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match declared_encoding(bytes).as_deref() {
        None | Some("utf-8" | "utf8" | "us-ascii" | "ascii") => {
            // UTF-8 invalide (flux mal étiqueté) : les octets fautifs deviennent « � ».
            Ok(String::from_utf8_lossy(bytes))
        }
        Some("iso-8859-1" | "latin1" | "latin-1" | "windows-1252" | "cp1252") => Ok(Cow::Owned(
            bytes
                .iter()
                .map(|&b| match b {
                    0x80..=0x9F => CP1252_HIGH[usize::from(b - 0x80)],
                    _ => char::from(b),
                })
                .collect(),
        )),
        Some(other) => Err(RssError::UnsupportedEncoding(truncate_chars(other, 40))),
    }
}

// ---------------------------------------------------------------------------------------
// Analyse
// ---------------------------------------------------------------------------------------

#[derive(Default)]
struct Enclosure {
    url: String,
    mime: String,
    length: Option<u64>,
}

#[derive(Default)]
struct ItemBuilder {
    title: Option<String>,
    description: Option<String>,
    summary: Option<String>,
    guid: Option<String>,
    pub_date: Option<String>,
    duration: Option<u32>,
    itunes_author: Option<String>,
    author: Option<String>,
    image: Option<String>,
    trailer: bool,
    enclosure: Option<Enclosure>,
}

#[derive(Default)]
struct FeedBuilder {
    title: Option<String>,
    description: Option<String>,
    summary: Option<String>,
    link: Option<String>,
    copyright: Option<String>,
    author: Option<String>,
    image: Option<String>,
    itunes_image: Option<String>,
}

struct State {
    stack: Vec<String>,
    /// Texte accumulé par élément ouvert (parallèle à `stack`).
    texts: Vec<String>,
    saw_root: bool,
    saw_channel: bool,
    feed: FeedBuilder,
    item: Option<ItemBuilder>,
    items_seen: usize,
    truncated: bool,
    seen_guids: HashSet<String>,
    episodes: Vec<PodcastEpisode>,
    warnings: Vec<String>,
    skipped_warnings: usize,
}

impl State {
    fn warn(&mut self, message: String) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(message);
        } else {
            self.skipped_warnings += 1;
        }
    }

    fn push_text(&mut self, s: &str) {
        if let Some(text) = self.texts.last_mut().filter(|t| t.len() < MAX_TEXT_BYTES) {
            text.push_str(s);
        }
    }

    /// Chemin courant : `rss/channel/item/title`…
    fn at(&self, depth: usize, name: &str) -> bool {
        self.stack.len() == depth && self.stack.last().is_some_and(|n| n == name)
    }

    fn in_item(&self) -> bool {
        self.stack.len() == 4 && self.stack[2] == "item"
    }

    fn in_channel(&self) -> bool {
        self.stack.len() == 3 && self.stack[1] == "channel"
    }

    fn start(&mut self, e: &BytesStart<'_>) -> Result<(), RssError> {
        let name = e.name().as_ref().to_ascii_lowercase();
        if self.stack.len() >= MAX_DEPTH {
            return Err(RssError::InvalidFeed(format!(
                "imbrication trop profonde (plus de {MAX_DEPTH} niveaux)"
            )));
        }
        if self.stack.is_empty() {
            if self.saw_root {
                return Err(RssError::InvalidFeed("plusieurs éléments racine".into()));
            }
            if name != "rss" {
                let shown = truncate_chars(&name, 40);
                return Err(RssError::InvalidFeed(format!(
                    "élément racine « {shown} » : un flux RSS 2.0 (« rss ») est attendu"
                )));
            }
            self.saw_root = true;
        }
        self.stack.push(name);
        self.texts.push(String::new());
        if self.stack.len() == 2 && self.stack[1] == "channel" {
            self.saw_channel = true;
        }
        if self.stack.len() == 3 && self.stack[1] == "channel" && self.stack[2] == "item" {
            self.item = Some(ItemBuilder::default());
            return Ok(());
        }
        let attr = |key: &str| -> Option<String> {
            e.try_get_attribute(key)
                .ok()
                .flatten()
                .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
                .map(|v| v.into_owned())
        };
        if self.at(4, "itunes:image") && self.in_item() {
            if let (Some(item), Some(href)) = (self.item.as_mut(), attr("href")) {
                item.image = item.image.take().or_else(|| http_url(&href));
            }
        } else if self.at(3, "itunes:image") && self.in_channel() {
            if let Some(href) = attr("href") {
                self.feed.itunes_image = self.feed.itunes_image.take().or_else(|| http_url(&href));
            }
        } else if self.at(4, "enclosure") && self.in_item() {
            if let Some(item) = self.item.as_mut() {
                let mime = attr("type").unwrap_or_default().trim().to_ascii_lowercase();
                let audio = mime.starts_with("audio/");
                // Première enclosure audio : les suivantes sont ignorées.
                if audio && item.enclosure.is_none() {
                    if let Some(url) = attr("url").as_deref().and_then(http_url) {
                        item.enclosure = Some(Enclosure {
                            url,
                            mime: truncate_chars(
                                &clean_line(&mime, MAX_MIME_CHARS),
                                MAX_MIME_CHARS,
                            ),
                            length: attr("length").and_then(|l| l.trim().parse().ok()),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn end(&mut self) {
        let raw = self.texts.pop().unwrap_or_default();
        let Some(name) = self.stack.last().cloned() else {
            return;
        };
        if self.in_item() {
            if let Some(item) = self.item.as_mut() {
                match name.as_str() {
                    "title" => {
                        item.title = item
                            .title
                            .take()
                            .or_else(|| non_empty(clean_line(&raw, MAX_TITLE_CHARS)))
                    }
                    "description" => {
                        item.description =
                            item.description.take().or_else(|| description_text(&raw))
                    }
                    "itunes:summary" => {
                        item.summary = item.summary.take().or_else(|| description_text(&raw))
                    }
                    "guid" => {
                        item.guid = item
                            .guid
                            .take()
                            .or_else(|| non_empty(clean_line(&raw, MAX_GUID_CHARS)))
                    }
                    "pubdate" => {
                        item.pub_date = item.pub_date.take().or_else(|| parse_pub_date(&raw))
                    }
                    "itunes:duration" => {
                        item.duration = item.duration.or_else(|| parse_duration(&raw))
                    }
                    "itunes:author" => {
                        item.itunes_author = item
                            .itunes_author
                            .take()
                            .or_else(|| non_empty(clean_line(&raw, MAX_AUTHOR_CHARS)))
                    }
                    "author" | "dc:creator" => {
                        item.author = item
                            .author
                            .take()
                            .or_else(|| non_empty(clean_line(&raw, MAX_AUTHOR_CHARS)))
                    }
                    "itunes:episodetype" => {
                        item.trailer = clean_line(&raw, 20).eq_ignore_ascii_case("trailer")
                    }
                    _ => {}
                }
            }
        } else if self.in_channel() && name != "item" {
            let f = &mut self.feed;
            match name.as_str() {
                "title" => {
                    f.title = f
                        .title
                        .take()
                        .or_else(|| non_empty(clean_line(&raw, MAX_TITLE_CHARS)))
                }
                "description" => {
                    f.description = f.description.take().or_else(|| description_text(&raw))
                }
                "itunes:summary" => f.summary = f.summary.take().or_else(|| description_text(&raw)),
                "link" => f.link = f.link.take().or_else(|| http_url(&raw)),
                "copyright" => {
                    f.copyright = f
                        .copyright
                        .take()
                        .or_else(|| non_empty(clean_line(&raw, MAX_AUTHOR_CHARS)))
                }
                "itunes:author" => {
                    f.author = f
                        .author
                        .take()
                        .or_else(|| non_empty(clean_line(&raw, MAX_AUTHOR_CHARS)))
                }
                _ => {}
            }
        } else if self.stack.len() == 4 && self.stack[2] == "image" && name == "url" {
            self.feed.image = self.feed.image.take().or_else(|| http_url(&raw));
        } else if self.stack.len() == 3 && name == "item" && self.stack[1] == "channel" {
            if let Some(item) = self.item.take() {
                self.finish_item(item);
            }
        }
        self.stack.pop();
        // Balises HTML non échappées dans un texte : leur contenu rejoint celui du parent.
        if self.stack.len() >= 3 && self.stack.last().is_some_and(|p| p != "item") {
            self.push_text(&raw);
        }
    }

    fn finish_item(&mut self, item: ItemBuilder) {
        self.items_seen += 1;
        let n = self.items_seen;
        if item.trailer {
            return self.warn(format!("épisode {n} ignoré : bande-annonce"));
        }
        let Some(enclosure) = item.enclosure else {
            return self.warn(format!(
                "épisode {n} ignoré : aucune enclosure audio valide"
            ));
        };
        if self.episodes.len() >= MAX_EPISODES {
            if !self.truncated {
                self.truncated = true;
                self.warn(format!(
                    "flux tronqué : épisodes au-delà de {MAX_EPISODES} ignorés"
                ));
            }
            return;
        }
        let guid = item.guid.unwrap_or_else(|| enclosure.url.clone());
        if !self.seen_guids.insert(guid.clone()) {
            return self.warn(format!("épisode {n} ignoré : identifiant en double"));
        }
        let title = item.title.unwrap_or_else(|| format!("Épisode {n}"));
        self.episodes.push(PodcastEpisode {
            guid,
            title,
            description: item.description.or(item.summary),
            pub_date: item.pub_date,
            duration: item.duration,
            author: item.itunes_author.or(item.author),
            image: item.image,
            enclosure_url: enclosure.url,
            enclosure_type: enclosure.mime,
            enclosure_length: enclosure.length,
        });
    }
}

/// Lit un flux RSS 2.0 + iTunes déjà téléchargé.
///
/// Tolérant sur le contenu, strict sur la sécurité : voir la documentation du module.
pub fn parse_feed(bytes: &[u8]) -> Result<PodcastFeed, RssError> {
    if bytes.len() > MAX_FEED_BYTES {
        return Err(RssError::FeedTooLarge {
            size: bytes.len(),
            max: MAX_FEED_BYTES,
        });
    }
    let text = decode_to_utf8(bytes)?;
    let mut reader = Reader::from_str(&text);
    reader.config_mut().check_end_names = true;
    let mut state = State {
        stack: Vec::new(),
        texts: Vec::new(),
        saw_root: false,
        saw_channel: false,
        feed: FeedBuilder::default(),
        item: None,
        items_seen: 0,
        truncated: false,
        seen_guids: HashSet::new(),
        episodes: Vec::new(),
        warnings: Vec::new(),
        skipped_warnings: 0,
    };
    loop {
        let event = reader
            .read_event()
            .map_err(|err| RssError::InvalidFeed(format!("XML mal formé : {err}")))?;
        match event {
            Event::Eof => break,
            Event::DocType(_) => return Err(RssError::DtdRefused),
            Event::Start(e) => state.start(&e)?,
            Event::Empty(e) => {
                state.start(&e)?;
                state.end();
            }
            Event::End(_) => state.end(),
            Event::Text(t) => state.push_text(&t.xml10_content()),
            Event::CData(c) => state.push_text(&c.xml10_content()),
            Event::GeneralRef(r) => match r.resolve_char_ref() {
                Ok(Some(c)) => {
                    let mut buf = [0u8; 4];
                    state.push_text(c.encode_utf8(&mut buf));
                }
                Ok(None) => match &*r {
                    "lt" => state.push_text("<"),
                    "gt" => state.push_text(">"),
                    "amp" => state.push_text("&"),
                    "quot" => state.push_text("\""),
                    "apos" => state.push_text("'"),
                    "nbsp" => state.push_text(" "),
                    // Entité inconnue : ignorée (aucune n'est définissable, DOCTYPE refusé).
                    _ => {}
                },
                Err(_) => {}
            },
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) => {}
        }
    }
    if !state.stack.is_empty() {
        return Err(RssError::InvalidFeed(
            "flux tronqué : éléments non fermés".into(),
        ));
    }
    if !state.saw_root || !state.saw_channel {
        return Err(RssError::InvalidFeed(
            "aucun canal (« channel ») dans le flux".into(),
        ));
    }
    if state.episodes.is_empty() {
        return Err(RssError::NoAudioEpisode);
    }
    if state.skipped_warnings > 0 {
        let skipped = state.skipped_warnings;
        state
            .warnings
            .push(format!("… et {skipped} autre(s) avertissement(s)"));
    }

    let f = state.feed;
    let channel_image = f
        .image
        .or(f.itunes_image)
        .or_else(|| state.episodes.iter().find_map(|e| e.image.clone()));
    let author_fallback = f.author.clone().or_else(|| f.copyright.clone());
    for episode in &mut state.episodes {
        if episode.image.is_none() {
            episode.image.clone_from(&channel_image);
        }
        if episode.author.is_none() {
            episode.author.clone_from(&author_fallback);
        }
    }
    Ok(PodcastFeed {
        title: f.title.unwrap_or_else(|| "Podcast sans titre".to_string()),
        description: f.description.or(f.summary),
        image: channel_image,
        link: f.link,
        copyright: f.copyright,
        episodes: state.episodes,
        warnings: state.warnings,
    })
}

#[cfg(test)]
mod tests;
