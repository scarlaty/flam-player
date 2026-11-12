//! Données qui ne sont ni un PNG ni un JPEG (décodé par [`crate::jpeg`] dès la signature
//! `FF D8 FF`) : Pillow pourrait-il les ouvrir ?
//!
//! `Image.open` essaie chacun de ses 43 greffons de lecture (45 avec `olefile`) : ceux qui ont une fonction
//! `_accept` ne voient que les 16 premiers octets, les autres (`IM`, `IMT`, `IPTC`, `PCD`,
//! `SPIDER`, `TGA`) sont ouverts directement et lèvent une exception sur un en-tête invalide.
//! Si aucun ne peut accepter les données, Pillow lève `UnidentifiedImageError` : Python émet
//! alors `image_unreadable` (ou `cover_unreadable`) et le Rust doit faire de même. Sinon,
//! c'est peut-être un autre format d'image que Pillow lirait : le Rust ne le décode pas et
//! s'arrête en `unsupported_format` (code 7, décision D4), **erreur finale** depuis le retrait
//! du repli Python (#138) qui arrête toute la conversion.
//!
//! La règle est donc **exacte là où elle est peu coûteuse** et prudente ailleurs. Les greffons
//! `IM`, `IMT`, `IPTC` et `TGA`, qui accepteraient sinon n'importe quel déchet (un fichier de
//! zéros, du texte, un SVG, une page HTML), reproduisent les contrôles de leur `_open` et ceux
//! d'`ImageFile.__init__` (mode vide, largeur ou hauteur nulle ou négative : « not identified
//! by this driver »), lus dans Pillow 12.0.0, `_open` identique en 12.3.0 (#574). Pour les
//! autres greffons, chaque condition
//! reprend `_accept` ou, pour quelques `_accept` qui rendent un message et pour `SPIDER`, une
//! condition plus large que le vrai contrôle. Un faux « peut-être » est une conversion qui
//! échoue alors que Pillow aurait rendu `image_unreadable` ; un faux « illisible » casserait
//! la parité (une image que Pillow lit remplacée par `empty.lif`), il n'y en a donc pas.
//!
//! Cas laissés du côté « peut-être » dans ces quatre greffons : valeur numérique d'un champ
//! `IM` ou `IMT` que Python lirait peut-être autrement qu'ici (soulignés `_` ; pour `IM`, dont
//! les valeurs sont décodées en Latin-1, aussi `\x1c`-`\x1f`, `\x85` et `\xa0`, que `str.strip()`
//! retire : réputée valide). Tout autre octet de contrôle ou d'au moins 0x80 rend la valeur
//! invalide (`ValueError`, #580) ; taille `IM` de trois nombres ou plus (le greffon
//! l'ouvre, le chargement échouerait plus tard). La garde anti-bombe de `Image.open`
//! (`2 × MAX_IMAGE_PIXELS`) est rejouée, comme les exceptions qu'elle ne rattrape pas
//! (`ValueError`, `OSError`) : elles rendent `image_unreadable` côté Python (`except Exception`).
//! Aucun contrôle d'un autre greffon n'est resserré : une signature `_accept` qui concorde
//! (par exemple `00 00 02 00`, CUR) reste « possible », même si le greffon refuserait ensuite
//! le reste de l'en-tête.

fn i16le(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

fn i16be(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

fn i32le(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

fn i32be(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

/// `_accept(prefix)` d'un des greffons qui en ont une (préfixe de 16 octets au plus). Les
/// greffons AVIF et WEBP sont pris au plus large (marque ou mode non vérifiés).
fn some_accept(p: &[u8]) -> bool {
    const MAGICS: &[&[u8]] = &[
        b"BLP1",
        b"BLP2",
        b"BM",
        b"BUFR",
        b"ZCZC",
        b"\0\0\x02\0",
        b"DDS ",
        b"%!PS",
        b"SIMPLE",
        b"FTEX",
        b"GIF87a",
        b"GIF89a",
        b"\x89HDF\r\n\x1a\n",
        b"\x89PNG\r\n\x1a\n",
        b"\xff\x4f\xff\x51",
        b"\0\0\0\x0cjP  \x0d\x0a\x87\x0a",
        b"icns",
        b"\0\0\x01\0",
        b"\xff\xd8\xff",
        b"\0\0\0\0\0\0\0\x04",
        b"\0\0\x01\xb3",
        b"MM\0*",
        b"II*\0",
        b"MM*\0",
        b"II\0*",
        b"MM\0+",
        b"II+\0",
        b"DanM",
        b"LinS",
        b"\x80\xe8\0\0",
        b"8BPS",
        b"qoif",
        b"\xd7\xcd\xc6\x9a\0\0",
        b"\x01\0\0\0",
        b"/* XPM */",
        b"P7 332",
        // FPX et MIC : greffons chargés seulement si `olefile` est installé (pas dans
        // l'installateur) ; pris quand même, par prudence.
        b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1",
    ];
    if MAGICS.iter().any(|m| p.starts_with(m)) {
        return true;
    }
    // AVIF : boîte `ftyp` (marque non vérifiée ici).
    let avif = p.get(4..8) == Some(b"ftyp");
    // WEBP : `RIFF` … `WEBP` (mode VP8 non vérifié ici).
    let webp = p.starts_with(b"RIFF") && p.get(8..12) == Some(b"WEBP");
    let dib = i32le(p, 0).is_some_and(|v| [12, 40, 52, 56, 64, 108, 124].contains(&v));
    let pcx = p.len() >= 2 && p[0] == 10 && [0, 2, 3, 5].contains(&p[1]);
    let dcx = i32le(p, 0) == Some(987_654_321);
    let eps = i32le(p, 0) == Some(0xC6D3_D0C5);
    let fli = p.len() >= 16
        && i16le(p, 4).is_some_and(|v| v == 0xAF11 || v == 0xAF12)
        && i16le(p, 14).is_some_and(|v| v == 0 || v == 3);
    let gbr = p.len() >= 8
        && i32be(p, 0).is_some_and(|v| v >= 20)
        && i32be(p, 4).is_some_and(|v| v == 1 || v == 2);
    let grib = p.len() >= 8 && p.starts_with(b"GRIB") && p[7] == 1;
    let ppm = p.len() >= 2 && p[0] == b'P' && b"0123456fy".contains(&p[1]);
    let sgi = i16be(p, 0) == Some(474);
    let sun = i32be(p, 0) == Some(0x59A6_6A95);
    // `bytes.lstrip()` : espaces ASCII de Python.
    let xbm = {
        let start = p
            .iter()
            .position(|b| !b" \t\n\r\x0b\x0c".contains(b))
            .unwrap_or(p.len());
        p[start..].starts_with(b"#define")
    };
    avif || webp || dib || pcx || dcx || eps || fli || gbr || grib || ppm || sgi || sun || xbm
}

/// `isInt` de `SpiderImagePlugin` pour un flottant 32 bits : fini et entier.
fn spider_int(f: f32) -> bool {
    f.is_finite() && f.fract() == 0.0
}

/// `isSpiderHeader`, sans le contrôle final `labbyt == labrec * lenbyt` (condition plus
/// large) : valeurs 1, 2, 5, 12, 13, 22, 23 entières et `iform` connu.
fn spider_header(header: &[u8], big_endian: bool) -> bool {
    let t = |i: usize| {
        let b: [u8; 4] = header[i * 4..i * 4 + 4].try_into().unwrap_or([0; 4]);
        if big_endian {
            f32::from_be_bytes(b)
        } else {
            f32::from_le_bytes(b)
        }
    };
    // h[i] = t[i - 1]
    [1usize, 2, 5, 12, 13, 22, 23]
        .iter()
        .all(|&i| spider_int(t(i - 1)))
        && [1.0, 3.0, -11.0, -12.0, -21.0, -22.0].contains(&t(4))
}

/// Façon dont Python lit la valeur d'un champ : `IM` décode la ligne en Latin-1 (`str`), `IMT`
/// passe les octets à `int()` (`bytes`).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Text {
    /// `str` : `strip()` retire aussi `\x1c`-`\x1f`, `\x85` et `\xa0` ; `_` sépare des chiffres.
    Latin1,
    /// `bytes` : seuls les espaces ASCII sont retirés ; `_` sépare des chiffres.
    Bytes,
}

impl Text {
    /// Octet que Python lirait peut-être comme faisant partie d'un nombre valide.
    fn unsure(self, b: u8) -> bool {
        b == b'_' || (self == Text::Latin1 && matches!(b, 0x1c..=0x1f | 0x85 | 0xa0))
    }

    /// Octet qui rend la valeur invalide (`ValueError`) sans exception possible : tout octet de
    /// contrôle hors espaces, `\x7f` et tout octet d'au moins 0x80, hormis ceux d'[`unsure`].
    fn invalid(self, b: u8) -> bool {
        !self.unsure(b) && (b >= 0x7f || (b < 0x20 && !PY_SPACE.contains(&b)))
    }
}

/// Résultat de la lecture d'un nombre comme le fait Python (`int` ou `float` sur un champ).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Num {
    Val(f64),
    /// `ValueError` certaine.
    Bad,
    /// Octets que Python lirait peut-être (soulignés, espaces Latin-1) : on ne tranche pas.
    Unsure,
}

/// Espaces que `int()` et `float()` retirent, restreints à l'ASCII.
const PY_SPACE: &[u8] = b" \t\n\r\x0b\x0c";

fn py_trim(t: &[u8]) -> &[u8] {
    let start = t
        .iter()
        .position(|b| !PY_SPACE.contains(b))
        .unwrap_or(t.len());
    let end = t
        .iter()
        .rposition(|b| !PY_SPACE.contains(b))
        .map_or(start, |i| i + 1);
    &t[start..end]
}

/// `int(t)` : signe facultatif puis chiffres ASCII.
fn py_int(t: &[u8], text: Text) -> Num {
    let t = py_trim(t);
    if t.iter().any(|&b| text.invalid(b)) {
        return Num::Bad;
    }
    if t.iter().any(|&b| text.unsure(b)) {
        return Num::Unsure;
    }
    let digits = t
        .strip_prefix(b"+")
        .or_else(|| t.strip_prefix(b"-"))
        .unwrap_or(t);
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Num::Bad;
    }
    std::str::from_utf8(t)
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .map_or(Num::Unsure, Num::Val)
}

/// `number(s)` d'`ImImagePlugin` : `int(s)`, sinon `float(s)`.
fn py_number(t: &[u8], text: Text) -> Num {
    match py_int(t, text) {
        Num::Bad => {}
        other => return other,
    }
    match std::str::from_utf8(py_trim(t))
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
    {
        Some(v) => Num::Val(v),
        None => Num::Bad,
    }
}

/// Valeur d'un champ `IM` de taille, de nombre d'images ou d'échelle : `v.replace("*", ",")`,
/// séparée par des virgules, chaque terme lu par `number`. `Err(true)` : valeur que Python
/// lirait peut-être ; `Err(false)` : `ValueError`.
fn im_numbers(v: &[u8]) -> Result<Vec<f64>, bool> {
    let mut out = Vec::new();
    for tok in v.split(|&b| b == b'*' || b == b',') {
        match py_number(tok, Text::Latin1) {
            Num::Val(x) => out.push(x),
            Num::Bad => return Err(false),
            Num::Unsure => return Err(true),
        }
    }
    Ok(out)
}

/// `x <= 0` de Python est faux pour NaN : seule une valeur nulle ou négative est rejetée.
fn positive(x: f64) -> bool {
    x.is_nan() || x > 0.0
}

/// `Image._decompression_bomb_check` : au-delà de `2 × MAX_IMAGE_PIXELS` pixels, `Image.open`
/// lève `DecompressionBombError` (non rattrapée : illisible).
fn bomb(w: f64, h: f64) -> bool {
    w.max(1.0) * h.max(1.0) > 2.0 * crate::png::MAX_IMAGE_PIXELS as f64
}

/// `TgaImageFile._open` : en-tête de 18 octets, champs valides, type d'image connu et, avec
/// palette, profondeur de palette connue.
fn tga_opens(d: &[u8]) -> bool {
    let Some(h) = d.get(..18) else { return false };
    let (colormap, imagetype, depth) = (h[1], h[2], h[16]);
    let width = u16::from_le_bytes([h[12], h[13]]);
    let height = u16::from_le_bytes([h[14], h[15]]);
    // « not a TGA file »
    if colormap > 1 || width == 0 || height == 0 || ![1, 8, 16, 24, 32].contains(&depth) {
        return false;
    }
    // « unknown TGA mode » : types 1, 2, 3 et leurs variantes RLE 9, 10, 11.
    if ![1, 2, 3, 9, 10, 11].contains(&imagetype) {
        return false;
    }
    // « unknown TGA map depth » ; les quatre orientations (bits 4 et 5) sont acceptées.
    (colormap == 0 || [16, 24, 32].contains(&h[7])) && !bomb(f64::from(width), f64::from(height))
}

/// `ImImageFile._open` : au moins un champ connu (`TAGS`), toutes les lignes de la forme
/// `Clé: valeur` et de 100 octets au plus, un octet `0x1A` (ou `0x00` puis plus loin `0x1A`) pour
/// finir l'en-tête, un mode non vide, une taille d'au moins deux nombres strictement positifs
/// et une table de 768 octets (au moins celle que Pillow lit) si le champ `Lut` est présent.
fn im_opens(d: &[u8]) -> bool {
    // Rejet immédiat sans saut de ligne dans les 100 premiers octets.
    if !d[..d.len().min(100)].contains(&b'\n') {
        return false;
    }
    const TAGS: [&[u8]; 9] = [
        b"Comment",
        b"Date",
        b"Digitalization equipment",
        b"File size (no of images)",
        b"Lut",
        b"Name",
        b"Scale (x,y)",
        b"Image size (x*y)",
        b"Image type",
    ];
    let mut pos = 0;
    let mut known = 0usize;
    let mut lut = false;
    let mut size = vec![512.0, 512.0];
    let mut mode_empty = false;
    let terminator = loop {
        let Some(&b) = d.get(pos) else { break None };
        pos += 1;
        if b == b'\r' {
            continue;
        }
        if b == 0 || b == 0x1A {
            break Some(b);
        }
        // `s + readline()`, puis retrait de `\r\n` ou de `\n`.
        let end = d[pos..]
            .iter()
            .position(|&c| c == b'\n')
            .map_or(d.len(), |i| pos + i + 1);
        let line = &d[pos - 1..end];
        pos = end;
        if line.len() > 100 {
            return false;
        }
        let line = line
            .strip_suffix(b"\r\n")
            .or_else(|| line.strip_suffix(b"\n"))
            .unwrap_or(line);
        // `^([A-Za-z][^:]*):[ \t]*(.*)[ \t]*$`
        let Some(colon) = line.iter().position(|&c| c == b':') else {
            return false;
        };
        if !line[0].is_ascii_alphabetic() {
            return false;
        }
        let key = &line[..colon];
        let value = &line[colon + 1..];
        let blanks = value
            .iter()
            .position(|&c| c != b' ' && c != b'\t')
            .unwrap_or(value.len());
        let value = &value[blanks..];
        match key {
            b"File size (no of images)" | b"Scale (x,y)" => {
                if im_numbers(value) == Err(false) {
                    return false; // `ValueError`
                }
            }
            b"Image size (x*y)" => match im_numbers(value) {
                Ok(v) => size = v,
                Err(false) => return false,
                // Valeur que Python lit peut-être : réputée valide, sans autre contrôle.
                Err(true) => size = vec![1.0, 1.0],
            },
            b"Image type" => mode_empty = value.is_empty(),
            b"Lut" => lut = true,
            _ => {}
        }
        if TAGS.contains(&key) {
            known += 1;
        }
    };
    // Sans aucun champ connu : « Not an IM file ».
    if known == 0 {
        return false;
    }
    // Fin de l'en-tête : `0x1A` à la dernière lecture, ou cherché plus loin après un `0x00` ;
    // sans lui, « File truncated ».
    let data_start = match terminator {
        Some(0x1A) => pos,
        Some(_) => match d[pos..].iter().position(|&c| c == 0x1A) {
            Some(i) => pos + i + 1,
            None => return false,
        },
        None => return false,
    };
    // La table de 768 octets lue après l'en-tête : `IndexError` si elle est trop courte pour la
    // comparaison chaînée `p[i] == p[i + 256] == p[i + 512]` (le troisième terme n'est lu que
    // si les deux premiers sont égaux).
    if lut {
        let p = &d[data_start..d.len().min(data_start + 768)];
        for i in 0..256 {
            let (Some(a), Some(b)) = (p.get(i), p.get(i + 256)) else {
                return false;
            };
            if a == b && p.get(i + 512).is_none() {
                return false;
            }
        }
    }
    // `ImageFile.__init__` : mode vide, ou `size[0]` / `size[1]` absent, nul ou négatif.
    !mode_empty
        && size.len() >= 2
        && positive(size[0])
        && positive(size[1])
        && !bomb(size[0], size[1])
}

/// `ImtImageFile._open` : la largeur, la hauteur et `pixel n8` (le seul mode que le greffon
/// définisse) doivent avoir été lues avant la fin de l'en-tête (octet `0x0C`, ou ligne que
/// le greffon ne comprend pas, qui l'arrête sans erreur).
fn imt_opens(d: &[u8]) -> bool {
    if !d[..d.len().min(100)].contains(&b'\n') {
        return false;
    }
    let (mut xsize, mut ysize, mut mode_l) = (0.0f64, 0.0f64, false);
    let mut pos = 0;
    while let Some(&b) = d.get(pos) {
        pos += 1;
        if b == 0x0C {
            break;
        }
        let end = d[pos..]
            .iter()
            .position(|&c| c == b'\n')
            .map_or(d.len(), |i| pos + i);
        let s = &d[pos - 1..end];
        pos = (end + 1).min(d.len());
        if s.len() == 1 || s.len() > 100 {
            break;
        }
        if s[0] == b'*' {
            continue; // commentaire
        }
        // `([a-z]*) ([^ \r\n]*)` au début de la ligne.
        let k_len = s
            .iter()
            .position(|c| !c.is_ascii_lowercase())
            .unwrap_or(s.len());
        if s.get(k_len) != Some(&b' ') {
            break;
        }
        let v = &s[k_len + 1..];
        let v = &v[..v
            .iter()
            .position(|&c| c == b' ' || c == b'\r')
            .unwrap_or(v.len())];
        let key = &s[..k_len];
        if key == b"width" || key == b"height" {
            let x = match py_int(v, Text::Bytes) {
                Num::Val(x) => x,
                Num::Bad => return false, // `ValueError`
                Num::Unsure => 1.0,       // Python lit peut-être la valeur : réputée valide
            };
            if key == b"width" {
                xsize = x;
            } else {
                ysize = x;
            }
        } else if key == b"pixel" && v == b"n8" {
            mode_l = true;
        }
    }
    mode_l && xsize > 0.0 && ysize > 0.0 && !bomb(xsize, ysize)
}

/// `IptcImageFile._open` : suite de champs `0x1C` (jeu 1 à 9 ou 240, longueur de 132 octets au
/// plus) qui finit sur cinq octets nuls, la fin du fichier ou le champ (8, 10) ; doivent y
/// figurer, une seule fois chacun, (3, 60) (couches et composante qui donnent un mode),
/// (3, 20) et (3, 30) (largeur et hauteur > 0) et (3, 120) (compression 1 ou 5).
fn iptc_opens(d: &[u8]) -> bool {
    // (3, 20), (3, 30), (3, 60), (3, 65), (3, 120) : nombre d'occurrences et dernières données.
    let mut seen: [(usize, Option<&[u8]>); 5] = [(0, None); 5];
    let mut pos = 0;
    loop {
        let s = &d[pos.min(d.len())..d.len().min(pos + 5)];
        pos += s.len();
        if s.iter().all(|&b| b == 0) {
            break;
        }
        // `s[1]`, `s[2]`, `s[3]` : `IndexError` sur un champ coupé.
        let (Some(&rec), Some(&ds), Some(&b3)) = (s.get(1), s.get(2), s.get(3)) else {
            return false;
        };
        if s[0] != 0x1C || ![1, 2, 3, 4, 5, 6, 7, 8, 9, 240].contains(&rec) {
            return false;
        }
        let size = match b3 {
            133.. => return false, // « illegal field length »
            128 => 0usize,
            129..=132 => {
                let n = usize::from(b3 - 128);
                let bytes = &d[pos.min(d.len())..d.len().min(pos + n)];
                pos += bytes.len();
                bytes.iter().fold(0usize, |a, &b| (a << 8) | usize::from(b))
            }
            _ => match s.get(4) {
                Some(&b4) => usize::from(b3) << 8 | usize::from(b4),
                None => return false, // `struct.error`
            },
        };
        if rec == 8 && ds == 10 {
            break;
        }
        let data = if size == 0 {
            None
        } else {
            let part = &d[pos.min(d.len())..d.len().min(pos.saturating_add(size))];
            pos += part.len();
            Some(part)
        };
        let slot = match (rec, ds) {
            (3, 20) => Some(0),
            (3, 30) => Some(1),
            (3, 60) => Some(2),
            (3, 65) => Some(3),
            (3, 120) => Some(4),
            _ => None,
        };
        if let Some(i) = slot {
            seen[i] = (seen[i].0 + 1, data);
        }
    }
    // Un champ absent, répété (Pillow en fait une liste) ou sans données lève `KeyError`,
    // `TypeError` ou `IndexError`.
    let get = |i: usize| match seen[i] {
        (1, Some(data)) => Some(data),
        _ => None,
    };
    // `_i` : les quatre derniers octets en gros-boutiste.
    let be = |data: &[u8]| {
        data[data.len().saturating_sub(4)..]
            .iter()
            .fold(0u64, |a, &b| (a << 8) | u64::from(b))
    };
    let Some(&[layers, component, ..]) = get(2) else {
        return false;
    };
    if layers == 1 && component == 0 {
        // mode `L`
    } else if (layers == 3 || layers == 4) && component != 0 {
        // RGB ou CMYK ; (3, 65), s'il existe, doit avoir au moins un octet.
        if seen[3].0 > 0 && get(3).is_none_or(<[u8]>::is_empty) {
            return false;
        }
    } else {
        return false; // mode vide : « not identified by this driver »
    }
    let (Some(w), Some(h), Some(c)) = (get(0), get(1), get(4)) else {
        return false;
    };
    let (w, h) = (be(w), be(h));
    w > 0 && h > 0 && [1, 5].contains(&be(c)) && !bomb(w as f64, h as f64)
}

/// Pillow pourrait-il ouvrir ces données (au moins un greffon les accepte) ?
pub fn pillow_may_open(data: &[u8]) -> bool {
    some_accept(&data[..data.len().min(16)])
        || im_opens(data)
        || imt_opens(data)
        || iptc_opens(data)
        || tga_opens(data)
        || spider_signature(data)
        || pcd_signature(data)
}

/// SPIDER (greffon sans `_accept`) : 27 flottants, gros-boutiste puis petit-boutiste.
pub(crate) fn spider_signature(data: &[u8]) -> bool {
    data.get(..108)
        .is_some_and(|h| spider_header(h, true) || spider_header(h, false))
}

/// PCD (greffon sans `_accept`) : `PCD_` à l'octet 2048.
pub(crate) fn pcd_signature(data: &[u8]) -> bool {
    data.get(2048..).is_some_and(|s| s.starts_with(b"PCD_"))
}

#[cfg(test)]
mod tests {
    use super::pillow_may_open;

    /// En-tête TGA de 18 octets : palette, type d'image, taille et profondeur. Longueur de
    /// l'identifiant à 5 : à 0 ou 1, les premiers octets pourraient former la signature `_accept`
    /// d'un autre greffon (`00 00 02 00` : CUR ; `01 00 00 00` : WMF) et le test ne passerait
    /// plus par les contrôles du TGA.
    fn tga(colormap: u8, imagetype: u8, w: u16, h: u16, depth: u8) -> Vec<u8> {
        let mut d = vec![0u8; 18];
        d[0] = 5;
        d[1] = colormap;
        d[2] = imagetype;
        d[12..14].copy_from_slice(&w.to_le_bytes());
        d[14..16].copy_from_slice(&h.to_le_bytes());
        d[16] = depth;
        d
    }

    /// Champ IPTC `0x1C`, jeu, numéro, longueur sur deux octets, données.
    fn iptc_field(rec: u8, ds: u8, data: &[u8]) -> Vec<u8> {
        let mut f = vec![0x1C, rec, ds];
        f.extend((data.len() as u16).to_be_bytes());
        f.extend(data);
        f
    }

    /// Flux IPTC complet : couches et composante, largeur, hauteur, compression.
    fn iptc(layers: &[u8], w: u8, h: u8, compression: u8) -> Vec<u8> {
        [
            iptc_field(3, 60, layers),
            iptc_field(3, 20, &[w]),
            iptc_field(3, 30, &[h]),
            iptc_field(3, 120, &[compression]),
        ]
        .concat()
    }

    /// Flux IPTC que Pillow ouvre : mode `L` (une couche), 1 × 1, compression brute.
    fn iptc_valid() -> Vec<u8> {
        iptc(&[1, 0], 1, 1, 1)
    }

    #[test]
    fn donnees_que_pillow_ne_reconnait_pas() {
        for d in [
            &b""[..],
            b"garbage",
            b"\x89PNGgarbage",
            b"\x89PNGgarb\0\0\0\x01",
            b"<html><body>404</body></html>",
            &[0xAA; 200],
        ] {
            assert!(!pillow_may_open(d), "{d:?}");
        }
    }

    /// #574 : des déchets que les greffons `TGA`, `IM`, `IMT` et `IPTC` rejettent au lieu de les
    /// ouvrir (`UnidentifiedImageError` : illisible, pas de délégation).
    #[test]
    fn dechets_rejetes_par_les_quatre_greffons() {
        let svg = b"<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\">\n  <rect/>\n</svg>\n";
        let html = b"<!DOCTYPE html>\n<html>\n<body>404 Not Found</body>\n</html>\n";
        let mut tga_zero_dims = tga(0, 2, 0, 1, 24);
        tga_zero_dims.extend([0xAA; 40]);
        for d in [
            vec![0u8; 18],
            vec![0u8; 19],
            vec![0u8; 1024],
            vec![1u8; 1024],
            // TGA : chaque champ invalide à son tour.
            tga(2, 2, 1, 1, 24),
            tga(0, 2, 0, 1, 24),
            tga(0, 2, 1, 0, 24),
            tga(0, 2, 1, 1, 12),
            tga(0, 0, 1, 1, 24),
            tga(0, 4, 1, 1, 24),
            tga(1, 1, 1, 1, 8), // palette sans profondeur de palette (octet 7 nul)
            tga_zero_dims,
            // Texte, SVG, HTML : un saut de ligne dans les 100 premiers octets suffisait.
            b"premiere ligne\ndeuxieme ligne\ntroisieme ligne\n".to_vec(),
            b"\n\n\n".to_vec(),
            svg.to_vec(),
            html.to_vec(),
            // IM : lignes bien formées, mais ni champ connu, ni `0x1A`, ou valeurs invalides.
            b"Auteur: x\nautre ligne sans deux-points\n".to_vec(),
            b"Auteur: x\n".to_vec(),
            b"Name: x\n".to_vec(),
            b"Name: x\nImage size (x*y): 0*5\n\x1a".to_vec(),
            b"Name: x\nImage size (x*y): 5\n\x1a".to_vec(),
            b"Name: x\nImage size (x*y): a*b\n\x1a".to_vec(),
            b"Name: x\nImage type:\n\x1a".to_vec(),
            b"Name: x\nLut: 1\n\x1a\x00\x01".to_vec(),
            // IMT : sans largeur, hauteur et `pixel n8` valides.
            b"width 4\nheight 4\n\x0c".to_vec(),
            b"width 0\nheight 4\npixel n8\n\x0c".to_vec(),
            b"width x\nheight 4\npixel n8\n\x0c".to_vec(),
            // IPTC : `0x1C` suivi de déchets, ou de champs qui n'en font pas une image.
            b"\x1c\x02\x00".to_vec(),
            b"\x1cgarbage and more garbage".to_vec(),
            vec![0x1C; 64],
            [&[0x1Cu8][..], &[0xAA; 40]].concat(),
            iptc_field(2, 0, b"titre"),
            iptc_field(3, 60, &[1, 0]),
            iptc(&[2, 0], 1, 1, 1),
            iptc(&[1, 0], 0, 1, 1),
            iptc(&[1, 0], 1, 1, 2),
            [iptc_valid(), iptc_field(3, 20, &[2])].concat(),
        ] {
            assert!(!pillow_may_open(&d), "{:?}", &d[..d.len().min(60)]);
        }
    }

    /// Les en-têtes que ces mêmes greffons ouvrent restent « possibles » : jamais un vrai
    /// fichier TGA, IM, IMT ou IPTC transformé en `image_unreadable`.
    #[test]
    fn vrais_en_tetes_des_quatre_greffons() {
        let mut tga_pal = tga(1, 1, 3, 2, 8);
        tga_pal[7] = 24;
        let mut im_lut = b"Name: x\nLut: 1\n\x1a".to_vec();
        im_lut.extend([0u8; 768]);
        for d in [
            // TGA : couleur, niveaux de gris, avec palette, RLE, profondeurs valides.
            tga(0, 2, 1, 1, 24),
            tga(0, 2, 1, 1, 32),
            tga(0, 10, 640, 480, 16),
            tga(0, 3, 2, 2, 8),
            tga(0, 3, 2, 2, 1),
            tga(0, 11, 2, 2, 16),
            tga(0, 1, 2, 2, 8),
            tga_pal,
            // IM : champs minimaux, fins de ligne `\r\n`, `0x00` puis `0x1A`, mode hors table.
            b"Name: x\n\x1a".to_vec(),
            b"Image type: RGB image\r\nImage size (x*y): 1*1\r\n\x1a\x00\x00\x00".to_vec(),
            b"Image size (x*y): 640*480\nImage type: L 8 image\n\x1a".to_vec(),
            b"Name: x\n\x00junk\x1a".to_vec(),
            b"Image type: Whatever\nName: x\n\x1a".to_vec(),
            b"Name: x\nImage size (x*y): 1.5*2\n\x1a".to_vec(),
            im_lut,
            // IMT : en-tête sur plusieurs lignes, commentaire.
            b"* commentaire\nwidth 4\nheight 3\npixel n8\n\x0c\x00".to_vec(),
            // IPTC : le flux minimal avec ou sans fin explicite, RGB, JPEG.
            iptc_valid(),
            [iptc_valid(), vec![0; 5]].concat(),
            [iptc_valid(), iptc_field(8, 10, b"")].concat(),
            iptc(&[3, 1], 1, 1, 5),
        ] {
            assert!(pillow_may_open(&d), "{:?}", &d[..d.len().min(60)]);
        }
    }

    /// Garde anti-bombe de `Image.open` : plus de `2 × MAX_IMAGE_PIXELS` (178 956 970) pixels
    /// est illisible (`DecompressionBombError`), au plus est possible ; NaN ne passe pas la
    /// comparaison de Python (`nan * 5` : pas de bombe).
    #[test]
    fn garde_anti_bombe() {
        let im = |size: &str| format!("Name: x\nImage size (x*y): {size}\n\x1a").into_bytes();
        let imt = |w: u32, h: u32| format!("width {w}\nheight {h}\npixel n8\n\x0c").into_bytes();
        assert!(pillow_may_open(&im("10000*10000")));
        assert!(!pillow_may_open(&im("20000*20000")));
        assert!(pillow_may_open(&im("nan*5")));
        assert!(pillow_may_open(&imt(10_000, 10_000)));
        assert!(!pillow_may_open(&imt(20_000, 20_000)));
        assert!(pillow_may_open(&tga(0, 2, 13_000, 13_000, 24)));
        assert!(!pillow_may_open(&tga(0, 2, 13_500, 13_500, 24)));
    }

    /// #580 : valeurs numériques que Pillow rejette (`ValueError`), jusque-là réputées valides.
    #[test]
    fn valeurs_numeriques_invalides() {
        const SPACES: &[u8] = b" \t\n\r\x0b\x0c";
        let imt = |w: &[u8]| [&b"width "[..], w, b"\nheight 3\npixel n8\n\x0c"].concat();
        let im = |v: &[u8]| [&b"Name: x\nImage size (x*y): "[..], v, b"\n\x1a"].concat();
        // IMT : octets de contrôle (hors espaces) et octets >= 0x80 : `int(bytes)` lève
        // `ValueError`.
        let mut bad_imt: Vec<Vec<u8>> = vec![b"4\x1a".to_vec(), b"4\x1f".to_vec()];
        for b in (0u8..0x20).filter(|b| !SPACES.contains(b)) {
            bad_imt.push(vec![b'4', b]);
        }
        bad_imt.extend([
            b"4\x7f".to_vec(),
            vec![b'4', 0x80],
            vec![0xff, b'4'],
            vec![b'4', 0x85],
            vec![b'4', 0xa0],
        ]);
        for w in &bad_imt {
            assert!(!pillow_may_open(&imt(w)), "IMT {w:?}");
        }
        // `_` sépare des chiffres (`int(b"4_0")` = 40) : reste possible.
        for w in [&b"4_0"[..], b"40", b"+4"] {
            assert!(pillow_may_open(&imt(w)), "IMT {w:?}");
        }
        // IM : valeur décodée en Latin-1. Contrôles hors espaces et octets >= 0x80 invalides,
        // sauf `_`, `\x1c`-`\x1f`, `\x85` et `\xa0`.
        let mut bad_im: Vec<Vec<u8>> = vec![b"1\x005*2".to_vec(), b"5*2\x01".to_vec()];
        for b in (0u8..0x20).filter(|b| !SPACES.contains(b) && !(0x1c..=0x1f).contains(b)) {
            bad_im.push(vec![b'5', b, b'*', b'5']);
        }
        for b in (0x7fu8..=0xff).filter(|b| ![0x85, 0xa0].contains(b)) {
            bad_im.push(vec![b'5', b, b'*', b'5']);
        }
        for v in &bad_im {
            assert!(!pillow_may_open(&im(v)), "IM {v:?}");
        }
        for v in [
            &b"1_0*5"[..],
            b"\x1c5*5",
            b"5\x1d*5",
            b"\x1e5*\x1f5",
            b"\x855*5",
            b"\xa05*5",
            b"5*5\xa0",
            b"640*480",
        ] {
            assert!(pillow_may_open(&im(v)), "IM {v:?}");
        }
    }

    #[test]
    fn formats_possibles_delegues() {
        // #574 : le texte multi-ligne, le TGA de zéros (seul le type d'image 2 posé) et
        // `\x1c\x02\x00` ne sont plus « possibles » : Pillow les rejette, le Rust aussi (voir
        // `dechets_rejetes_par_les_quatre_greffons`). Un vrai en-tête TGA, IM et IPTC les
        // remplace ; IMT est dans `vrais_en_tetes_des_quatre_greffons`.
        let tga = tga(0, 2, 1, 1, 24);
        for d in [
            &b"\xff\xd8\xff\xe0JFIF"[..],
            b"GIF89a\x01\x00",
            b"BM\0\0",
            b"P1 1 1 1",
            b"RIFF\0\0\0\0WEBPVP8 ",
            b"  #define w 1",
            b"Name: texte\n\x1a",
            &tga,
            &iptc_valid(),
        ] {
            assert!(pillow_may_open(d), "{d:?}");
        }
        let mut pcd = vec![0xAAu8; 2052];
        pcd[2048..].copy_from_slice(b"PCD_");
        assert!(pillow_may_open(&pcd));
        // SPIDER (gros-boutiste) : flottants entiers, iform = -11 ; sans iform connu, rejet.
        let spider = |iform: f32| -> Vec<u8> {
            (0..27)
                .flat_map(|i| if i == 4 { iform } else { 3.0f32 }.to_be_bytes())
                .collect()
        };
        assert!(pillow_may_open(&spider(-11.0)));
        assert!(!pillow_may_open(&spider(7.0)));
    }
}
