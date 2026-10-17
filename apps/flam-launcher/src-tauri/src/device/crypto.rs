//! Chiffrement des histoires pour la Flam (firmware 2.x, #228), logique pure sans Tauri.
//!
//! Format attendu par la Flam (observé, à confirmer sur appareil par #236) :
//!
//! - les scripts (`*.lsf`) et le fichier `info` d'une histoire sont chiffrés en AES-128-CBC sur
//!   toute leur longueur ; la fin est complétée par des octets nuls jusqu'au multiple de
//!   16 octets suivant (aucun bloc ajouté si la longueur est déjà alignée, un fichier vide reste
//!   vide) ;
//! - la clé et l'IV de ce chiffrement sont propres à l'appareil et se lisent dans `.mdf` à la
//!   racine de la Flam (voir [`DeviceKeys`]) ;
//! - le fichier `key` de l'histoire (32 octets) se déduit du numéro de série de l'appareil
//!   (voir [`DeviceKeys::key_file`]).
//!
//! Seul le firmware 2.x est pris en charge ici ; le 1.x relève de #234.
//!
//! Les clés ne sont jamais journalisées : `Debug` les masque, et elles sont effacées de la
//! mémoire à la destruction (`zeroize`, y compris la clé étendue d'AES).

use aes::cipher::{block_padding::NoPadding, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use zeroize::Zeroizing;

/// Taille d'un bloc AES, en octets.
pub const BLOCK_SIZE: usize = 16;
/// Taille du numéro de série binaire de la Flam, en octets.
pub const SERIAL_LEN: usize = 7;
/// Taille du fichier `key` d'une histoire, en octets.
pub const KEY_FILE_LEN: usize = 32;
/// Nombre d'octets nuls entre les deux copies du numéro de série dans le fichier `key`.
const KEY_FILE_GAP: usize = 10;
/// Nombre de caractères hexadécimaux du numéro de série répétés en fin de fichier `key`.
const KEY_FILE_TAIL: usize = 8;

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// Erreur de chiffrement ou de clés d'appareil.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    /// La longueur des données n'est pas un multiple de [`BLOCK_SIZE`].
    #[error("longueur {0} non multiple de 16 octets")]
    UnalignedLength(usize),
    /// Version de firmware non prise en charge (seul le 2.x l'est).
    #[error("firmware de la Flam non pris en charge : « {0} » (seul le 2.x l'est)")]
    UnsupportedFirmware(String),
    /// Numéro de série de longueur inattendue.
    #[error("numéro de série de la Flam invalide : {0} octets au lieu de 7")]
    InvalidSerial(usize),
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

/// Longueur de `len` octets complétés par des zéros jusqu'au multiple de 16 suivant.
pub fn zero_padded_len(len: usize) -> usize {
    len.div_ceil(BLOCK_SIZE) * BLOCK_SIZE
}

/// Chiffre `plain` au format de la Flam : complété par des octets nuls jusqu'au multiple de
/// 16 octets suivant, puis AES-128-CBC. Le résultat fait [`zero_padded_len`] octets.
pub fn encrypt_zero_padded(key: &[u8; 16], iv: &[u8; 16], plain: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(zero_padded_len(plain.len()));
    out.extend_from_slice(plain);
    out.resize(zero_padded_len(plain.len()), 0);
    encrypt_blocks(key, iv, &mut out).expect("longueur alignée par construction");
    out
}

/// Version de firmware prise en charge, déduite du champ « main » de `.mdf` (`2.4.2`…).
fn check_firmware(firmware_main: &str) -> Result<(), CryptoError> {
    let major = firmware_main.trim().split('.').next().unwrap_or("");
    match major.parse::<u32>() {
        Ok(2) => Ok(()),
        _ => Err(CryptoError::UnsupportedFirmware(firmware_main.to_string())),
    }
}

/// Clés d'une Flam en firmware 2.x, telles que lues dans son fichier `.mdf`.
///
/// Interface minimale remplie par la détection (#225). Disposition observée de `.mdf` :
/// 2 octets d'en-tête, 48 octets de texte (« main: x.y.z-… » puis « comm: … », complété par
/// des nuls), 24 caractères ASCII hexadécimaux du numéro de série (complétés par des nuls ; le
/// numéro décodé fait 7 octets), VID et PID (2 octets chacun, petit-boutiste), puis à
/// l'octet `0x4E` la clé (16 octets) et l'IV (16 octets) de chiffrement des histoires.
pub struct DeviceKeys {
    serial: [u8; SERIAL_LEN],
    story_key: Zeroizing<[u8; 16]>,
    story_iv: Zeroizing<[u8; 16]>,
}

impl DeviceKeys {
    /// Construit les clés d'un appareil. `firmware_main` est la version principale
    /// (« 2.4.2 ») ; `serial` le numéro de série binaire (hexadécimal de `.mdf` décodé).
    ///
    /// Erreur [`CryptoError::UnsupportedFirmware`] hors firmware 2.x : aucune préparation
    /// n'est alors possible.
    pub fn new(
        firmware_main: &str,
        serial: &[u8],
        story_key: [u8; 16],
        story_iv: [u8; 16],
    ) -> Result<Self, CryptoError> {
        let story_key = Zeroizing::new(story_key);
        let story_iv = Zeroizing::new(story_iv);
        check_firmware(firmware_main)?;
        let serial: [u8; SERIAL_LEN] = serial
            .try_into()
            .map_err(|_| CryptoError::InvalidSerial(serial.len()))?;
        Ok(Self {
            serial,
            story_key,
            story_iv,
        })
    }

    /// Chiffre un script ou le fichier `info` d'une histoire pour cet appareil.
    pub fn encrypt(&self, plain: &[u8]) -> Vec<u8> {
        encrypt_zero_padded(&self.story_key, &self.story_iv, plain)
    }

    /// Contenu du fichier `key` d'une histoire (32 octets) : le numéro de série en
    /// hexadécimal minuscule ASCII (14 caractères), 10 octets nuls, puis les 8 premiers
    /// caractères de ce même hexadécimal.
    pub fn key_file(&self) -> Zeroizing<[u8; KEY_FILE_LEN]> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut hex = Zeroizing::new([0u8; 2 * SERIAL_LEN]);
        for (i, b) in self.serial.iter().enumerate() {
            hex[2 * i] = HEX[usize::from(b >> 4)];
            hex[2 * i + 1] = HEX[usize::from(b & 0x0f)];
        }
        let mut out = Zeroizing::new([0u8; KEY_FILE_LEN]);
        out[..hex.len()].copy_from_slice(&hex[..]);
        let tail = hex.len() + KEY_FILE_GAP;
        out[tail..].copy_from_slice(&hex[..KEY_FILE_TAIL]);
        out
    }
}

impl std::fmt::Debug for DeviceKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeviceKeys { <masqué> }")
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn hex(s: &str) -> Vec<u8> {
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

    /// Clés de test inventées (aucune vraie clé d'appareil).
    pub(crate) const TEST_KEY: [u8; 16] = [
        0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae,
        0xaf,
    ];
    pub(crate) const TEST_IV: [u8; 16] = [
        0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x5b, 0x5c, 0x5d, 0x5e,
        0x5f,
    ];
    pub(crate) const TEST_SERIAL: [u8; 7] = [0x00, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc];

    pub(crate) fn test_keys() -> DeviceKeys {
        DeviceKeys::new("2.4.2", &TEST_SERIAL, TEST_KEY, TEST_IV).unwrap()
    }

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

    /// Références calculées par une implémentation indépendante (.NET
    /// `System.Security.Cryptography.Aes`, CBC, `PaddingMode.Zeros`) avec les clés de test.
    #[test]
    fn bourrage_nul_conforme_a_une_reference_independante() {
        let cases: [(&[u8], &str); 3] = [
            (
                b"Titre\nSous-titre\n000000\nTitre\n",
                "87ee5c7ac9536c8d4ba0687ed1981bc153eceb0b54e30198170fbb35302177bc",
            ),
            (b"0123456789abcdef", "eb6752b6bd081c82e9fe8192900e8368"),
            (b"x", "97e69e488ff4bc1f8f8476c3f0173fbb"),
        ];
        for (plain, expected) in cases {
            assert_eq!(
                encrypt_zero_padded(&TEST_KEY, &TEST_IV, plain),
                hex(expected)
            );
        }
        assert!(encrypt_zero_padded(&TEST_KEY, &TEST_IV, b"").is_empty());
    }

    #[test]
    fn aller_retour_avec_bourrage_pour_toutes_les_longueurs() {
        for len in 0..=49usize {
            let plain: Vec<u8> = (0..len).map(|i| (i as u8).wrapping_mul(37) | 1).collect();
            let mut data = test_keys().encrypt(&plain);
            assert_eq!(data.len(), zero_padded_len(len), "longueur {len}");
            decrypt_blocks(&TEST_KEY, &TEST_IV, &mut data).unwrap();
            assert_eq!(&data[..len], &plain[..]);
            assert!(data[len..].iter().all(|&b| b == 0));
        }
    }

    #[test]
    fn fichier_key_derive_du_numero_de_serie() {
        let key = test_keys().key_file();
        let mut expected = b"00123456789abc".to_vec();
        expected.extend_from_slice(&[0u8; 10]);
        expected.extend_from_slice(b"00123456");
        assert_eq!(&key[..], &expected[..]);
    }

    #[test]
    fn firmware_hors_2x_refuse() {
        for fw in ["1.15.14", "3.0.0", "", "abc", "20.1"] {
            assert_eq!(
                DeviceKeys::new(fw, &TEST_SERIAL, TEST_KEY, TEST_IV).unwrap_err(),
                CryptoError::UnsupportedFirmware(fw.to_string())
            );
        }
        assert!(DeviceKeys::new("2.0.0", &TEST_SERIAL, TEST_KEY, TEST_IV).is_ok());
    }

    #[test]
    fn numero_de_serie_de_mauvaise_taille_refuse() {
        assert_eq!(
            DeviceKeys::new("2.4.2", &[1, 2, 3], TEST_KEY, TEST_IV).unwrap_err(),
            CryptoError::InvalidSerial(3)
        );
    }

    #[test]
    fn debug_ne_montre_aucune_cle() {
        let shown = format!("{:?} {:#?}", test_keys(), test_keys());
        for secret in [&TEST_KEY[..], &TEST_IV[..], &TEST_SERIAL[..]] {
            let as_hex: String = secret.iter().map(|b| format!("{b:02x}")).collect();
            assert!(!shown.to_lowercase().contains(&as_hex), "{shown}");
            assert!(!shown.contains(&format!("{:?}", secret)), "{shown}");
        }
    }
}
