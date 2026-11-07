//! Outils communs des tests : décodeur RLE4 minimal et indépendant du code testé.
#![allow(dead_code)]

pub const W: usize = 320;
pub const H: usize = 240;

/// Découpe un BMP RLE4 de l'appareil : vérifie l'en-tête exact et renvoie les niveaux 0..=15,
/// lignes de haut en bas. Refuse toute structure RLE différente de celle de Lunii.QT (séries
/// codées seulement, fin de ligne `00 00` sauf après la dernière, fin `00 01`).
pub fn decode_device_bmp(b: &[u8]) -> Result<Vec<u8>, String> {
    if b.len() < 118 {
        return Err("trop court".into());
    }
    let u32at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let u16at = |o: usize| u16::from_le_bytes(b[o..o + 2].try_into().unwrap());
    if &b[..2] != b"BM" {
        return Err("signature".into());
    }
    let n = b.len() - 118;
    let checks = [
        (u32at(2) as usize == b.len(), "taille fichier"),
        (u32at(6) == 0, "réservé"),
        (u32at(10) == 118, "offset"),
        (u32at(14) == 40, "taille en-tête"),
        (u32at(18) == 320, "largeur"),
        (u32at(22) == 240, "hauteur"),
        (u16at(26) == 1, "plans"),
        (u16at(28) == 4, "bits"),
        (u32at(30) == 2, "compression"),
        (u32at(34) as usize == n, "taille données"),
        (u32at(38) == 0 && u32at(42) == 0, "résolution"),
        (u32at(46) == 16, "couleurs"),
        (u32at(50) == 0, "importantes"),
    ];
    for (ok, what) in checks {
        if !ok {
            return Err(format!("en-tête : {what}"));
        }
    }
    for i in 0..16usize {
        let g = (255 * i / 16) as u8;
        if b[54 + i * 4..58 + i * 4] != [g, g, g, 0] {
            return Err(format!("palette {i}"));
        }
    }
    let d = &b[118..];
    let mut rows: Vec<Vec<u8>> = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut i = 0;
    loop {
        let a = *d.get(i).ok_or("fin inattendue")?;
        let v = *d.get(i + 1).ok_or("fin inattendue")?;
        i += 2;
        if a == 0 {
            match v {
                0 => {
                    if cur.len() != W {
                        return Err(format!("ligne de {} pixels", cur.len()));
                    }
                    rows.push(std::mem::take(&mut cur));
                }
                1 => {
                    if cur.len() != W {
                        return Err(format!("dernière ligne de {} pixels", cur.len()));
                    }
                    rows.push(std::mem::take(&mut cur));
                    break;
                }
                _ => return Err("code d'échappement inattendu".into()),
            }
        } else {
            if v >> 4 != v & 15 {
                return Err("demi-octets différents".into());
            }
            cur.extend(std::iter::repeat_n(v & 15, a as usize));
        }
    }
    if i != d.len() {
        return Err("octets après la fin".into());
    }
    if rows.len() != H {
        return Err(format!("{} lignes", rows.len()));
    }
    // Stocké de bas en haut.
    Ok(rows.into_iter().rev().flatten().collect())
}
