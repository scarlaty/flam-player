//! Chiffrement AES-128-CBC (#228), logique pure sans Tauri.
//!
//! Primitive standard (NIST SP 800-38A) sur des blocs entiers de 16 octets, via les crates
//! RustCrypto `aes` et `cbc`. Le bourrage, le choix des fichiers chiffrés et la gestion des clés
//! d'appareil viendront dans une PR ultérieure, une fois la question de licence tranchée.

use aes::cipher::{block_padding::NoPadding, BlockDecryptMut, BlockEncryptMut, KeyIvInit};

/// Taille d'un bloc AES, en octets.
pub const BLOCK_SIZE: usize = 16;

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// Erreur de chiffrement.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    /// La longueur des données n'est pas un multiple de [`BLOCK_SIZE`].
    #[error("longueur {0} non multiple de 16 octets")]
    UnalignedLength(usize),
}

fn check_aligned(data: &[u8]) -> Result<(), CryptoError> {
    if data.len().is_multiple_of(BLOCK_SIZE) {
        Ok(())
    } else {
        Err(CryptoError::UnalignedLength(data.len()))
    }
}

/// Chiffre `data` en place (longueur multiple de 16, aucun bourrage).
pub fn encrypt_blocks(key: &[u8; 16], iv: &[u8; 16], data: &mut [u8]) -> Result<(), CryptoError> {
    check_aligned(data)?;
    let len = data.len();
    Aes128CbcEnc::new(key.into(), iv.into())
        .encrypt_padded_mut::<NoPadding>(data, len)
        .map_err(|_| CryptoError::UnalignedLength(len))?;
    Ok(())
}

/// Déchiffre `data` en place (longueur multiple de 16, aucun bourrage).
pub fn decrypt_blocks(key: &[u8; 16], iv: &[u8; 16], data: &mut [u8]) -> Result<(), CryptoError> {
    check_aligned(data)?;
    let len = data.len();
    Aes128CbcDec::new(key.into(), iv.into())
        .decrypt_padded_mut::<NoPadding>(data)
        .map_err(|_| CryptoError::UnalignedLength(len))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn arr16(s: &str) -> [u8; 16] {
        hex(s).try_into().unwrap()
    }

    // NIST SP 800-38A, annexe F.2.1 / F.2.2 (CBC-AES128).
    const KEY: &str = "2b7e151628aed2a6abf7158809cf4f3c";
    const IV: &str = "000102030405060708090a0b0c0d0e0f";
    const PLAIN: &str = concat!(
        "6bc1bee22e409f96e93d7e117393172a",
        "ae2d8a571e03ac9c9eb76fac45af8e51",
        "30c81c46a35ce411e5fbc1191a0a52ef",
        "f69f2445df4f9b17ad2b417be66c3710"
    );
    const CIPHER: &str = concat!(
        "7649abac8119b246cee98e9b12e9197d",
        "5086cb9b507219ee95db113a917678b2",
        "73bed6b8e3c1743b7116e69e22229516",
        "3ff1caa1681fac09120eca307586e1a7"
    );

    #[test]
    fn vecteur_nist_chiffrement() {
        let mut data = hex(PLAIN);
        encrypt_blocks(&arr16(KEY), &arr16(IV), &mut data).unwrap();
        assert_eq!(data, hex(CIPHER));
    }

    #[test]
    fn vecteur_nist_dechiffrement() {
        let mut data = hex(CIPHER);
        decrypt_blocks(&arr16(KEY), &arr16(IV), &mut data).unwrap();
        assert_eq!(data, hex(PLAIN));
    }

    #[test]
    fn aller_retour_et_vide() {
        let key = [0x11u8; 16];
        let iv = [0x22u8; 16];
        let orig: Vec<u8> = (0..=255u8).collect();
        let mut data = orig.clone();
        encrypt_blocks(&key, &iv, &mut data).unwrap();
        assert_ne!(data, orig);
        decrypt_blocks(&key, &iv, &mut data).unwrap();
        assert_eq!(data, orig);
        let mut empty: Vec<u8> = Vec::new();
        encrypt_blocks(&key, &iv, &mut empty).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn longueur_non_alignee_refusee() {
        let mut data = vec![0u8; 17];
        let before = data.clone();
        assert_eq!(
            encrypt_blocks(&[0; 16], &[0; 16], &mut data),
            Err(CryptoError::UnalignedLength(17))
        );
        assert_eq!(data, before);
        assert_eq!(
            decrypt_blocks(&[0; 16], &[0; 16], &mut data[..15]),
            Err(CryptoError::UnalignedLength(15))
        );
    }
}
