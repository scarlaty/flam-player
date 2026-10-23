//! Données qui ne sont pas un PNG : Pillow 12.3.0 pourrait-il les ouvrir ?
//!
//! `Image.open` essaie chacun de ses 43 greffons de lecture (45 avec `olefile`) : ceux qui ont une fonction
//! `_accept` ne voient que les 16 premiers octets, les autres (`IM`, `IMT`, `IPTC`, `PCD`,
//! `SPIDER`, `TGA`) sont ouverts directement et lèvent une exception sur un en-tête invalide.
//! Si aucun ne peut accepter les données, Pillow lève `UnidentifiedImageError` : Python émet
//! alors `image_unreadable` (ou `cover_unreadable`) et le Rust doit faire de même. Sinon,
//! c'est peut-être un autre format d'image que Pillow lirait : délégation à Python
//! (`unsupported_format`, code 7, décision D4).
//!
//! La règle est **prudente** : chaque condition reprend exactement `_accept` (Pillow 12.3.0)
//! ou, pour les greffons sans `_accept` et quelques `_accept` qui rendent un message, une
//! condition plus large que le vrai contrôle. Un faux « peut-être » ne coûte qu'une délégation ;
//! un faux « illisible » casserait la parité, il n'y en a donc pas.

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

/// Pillow pourrait-il ouvrir ces données (au moins un greffon les accepte) ?
pub fn pillow_may_open(data: &[u8]) -> bool {
    let prefix = &data[..data.len().min(16)];
    if some_accept(prefix) {
        return true;
    }
    // IM et IMT : rejet immédiat sans saut de ligne dans les 100 premiers octets.
    if data[..data.len().min(100)].contains(&b'\n') {
        return true;
    }
    // IPTC : premier champ `0x1C` (un en-tête nul lève `KeyError`, qui interrompt aussi
    // `Image.open` : illisible dans tous les cas).
    if data.first() == Some(&0x1C) {
        return true;
    }
    // TGA : en-tête de 18 octets, type de palette 0 ou 1.
    if data.len() >= 18 && data[1] <= 1 {
        return true;
    }
    // SPIDER : 27 flottants, gros-boutiste puis petit-boutiste.
    if let Some(h) = data.get(..108) {
        if spider_header(h, true) || spider_header(h, false) {
            return true;
        }
    }
    // PCD : `PCD_` à l'octet 2048.
    data.get(2048..).is_some_and(|s| s.starts_with(b"PCD_"))
}

#[cfg(test)]
mod tests {
    use super::pillow_may_open;

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

    #[test]
    fn formats_possibles_delegues() {
        let mut tga = vec![0u8; 18];
        tga[2] = 2;
        for d in [
            &b"\xff\xd8\xff\xe0JFIF"[..],
            b"GIF89a\x01\x00",
            b"BM\0\0",
            b"P1 1 1 1",
            b"RIFF\0\0\0\0WEBPVP8 ",
            b"  #define w 1",
            b"text\nwith newline",
            &tga,
            b"\x1c\x02\x00",
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
