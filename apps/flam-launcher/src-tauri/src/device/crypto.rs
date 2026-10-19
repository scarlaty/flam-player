//! Chiffrement des histoires pour la Flam (firmware 2.x, #228 ; firmware 1.x par « carrier »,
//! #234), logique pure sans Tauri.
//!
//! Format attendu par la Flam (observé, à confirmer sur appareil par #236) :
//!
//! - les scripts (`*.lsf`) et le fichier `info` d'une histoire sont chiffrés en AES-128-CBC sur
//!   toute leur longueur ; la fin est complétée par des octets nuls jusqu'au multiple de
//!   16 octets suivant (aucun bloc ajouté si la longueur est déjà alignée, un fichier vide reste
//!   vide) ;
//! - la clé et l'IV de ce chiffrement sont propres à l'appareil et se lisent dans `.mdf` à la
//!   racine de la Flam (disposition ci-dessous) ;
//! - le fichier `key` de l'histoire (32 octets) se déduit du numéro de série de l'appareil
//!   (voir [`DeviceKeys::key_file`]).
//!
//! # Disposition de `.mdf` (126 octets)
//!
//! | Offset | Taille | Contenu                                                                |
//! |--------|--------|------------------------------------------------------------------------|
//! | `0x00` | 2      | en-tête, non lu ici (rôle à confirmer sur appareil, #236)              |
//! | `0x02` | 48     | version « main: x.y.z-… » (2.x) ou nue « x.y.z-… » (1.x), octets nuls  |
//! | `0x32` | 24     | numéro de série : 14 caractères ASCII hexadécimaux puis 10 octets nuls |
//! | `0x4A` | 2      | VID USB (petit-boutiste)                                               |
//! | `0x4C` | 2      | PID USB (petit-boutiste)                                               |
//! | `0x4E` | 16     | firmware 2.x : clé AES des histoires                                   |
//! | `0x5E` | 16     | firmware 2.x : IV AES des histoires                                    |
//! | `0x6E` | 16     | non lu ici ; rôle inconnu, à confirmer sur appareil (#236)             |
//!
//! La taille totale ([`MDF_LEN`], 126 octets) est celle mesurée sur l'appareil de l'inventaire ;
//! sa validation relève de la détection (#241). Ce module se sert du texte de version, du numéro
//! de série, de la clé et de l'IV ; VID et PID sont lus par la détection. Le
//! numéro de série se décode avec [`parse_serial_field`].
//!
//! # Firmwares
//!
//! La version est le champ « main » (nombre avant le premier point). Le firmware 2.x lit ses
//! clés dans `.mdf` ([`DeviceKeys::new`]). Le 1.x range à `0x4E` 32 octets d'une autre nature
//! (ni clé ni IV) : il n'accepte que les clés « carrier » d'une histoire officielle du même
//! compte ([`DeviceKeys::with_carrier`], module [`super::carrier`], #234). Les firmwares 3.x et
//! suivants n'ont jamais été observés : ils sont refusés explicitement plutôt que de produire
//! un fichier `key` douteux (à confirmer sur appareil, #236).
//!
//! Les clés ne sont jamais journalisées : `Debug` les masque, et elles sont effacées de la
//! mémoire à la destruction (`zeroize`, y compris la clé étendue d'AES).

use aes::cipher::{block_padding::NoPadding, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use zeroize::Zeroizing;

use super::carrier::CarrierKeys;

/// Taille d'un bloc AES, en octets.
pub const BLOCK_SIZE: usize = 16;
/// Taille du numéro de série binaire de la Flam, en octets (14 caractères hexadécimaux).
pub const SERIAL_LEN: usize = 7;
/// Taille du champ du numéro de série dans `.mdf` (caractères hexadécimaux puis octets nuls).
pub const SERIAL_FIELD_LEN: usize = 24;
/// Taille totale de `.mdf` mesurée sur appareil (celle de la détection, #241).
pub const MDF_LEN: usize = super::detect::MDF_DOCUMENTED_SIZE as usize;
/// Offset du champ du numéro de série dans `.mdf`.
pub const MDF_SERIAL_OFFSET: usize = 0x32;
/// Offset de la clé AES des histoires dans `.mdf` (firmware 2.x).
pub const MDF_KEY_OFFSET: usize = 0x4E;
/// Offset de l'IV AES des histoires dans `.mdf` (firmware 2.x).
pub const MDF_IV_OFFSET: usize = 0x5E;
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
    /// Version de firmware non prise en charge : 3.x et suivants (jamais observés), ou famille
    /// inattendue pour la source des clés (1.x sans clés carrier, 2.x avec).
    #[error(
        "firmware de la Flam non pris en charge : « {0} » (2.x, ou 1.x avec les clés d'une \
         histoire officielle ; 3.x et suivants non vérifiés)"
    )]
    UnsupportedFirmware(String),
    /// Numéro de série de longueur inattendue.
    #[error("numéro de série de la Flam invalide : {0} octets au lieu de 7")]
    InvalidSerial(usize),
    /// Champ du numéro de série de `.mdf` mal formé. Le contenu n'est pas repris dans le
    /// message : c'est une donnée propre à l'appareil.
    #[error("numéro de série de la Flam illisible dans .mdf : {0}")]
    MalformedSerialField(&'static str),
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

/// Famille de firmware reconnue d'après le champ « main » de `.mdf`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareFamily {
    /// 1.x : clés « carrier » obligatoires (#234).
    V1,
    /// 2.x : clés lues dans `.mdf`.
    V2,
}

/// Famille du firmware `firmware_main` (`2.4.2`, `1.15.14`…), `None` hors 1.x et 2.x (3.x et
/// suivants, texte illisible). Majeur en chiffres ASCII seulement : ni signe, ni espace, ni
/// zéro initial.
pub fn firmware_family(firmware_main: &str) -> Option<FirmwareFamily> {
    match firmware_main.split('.').next() {
        Some("1") => Some(FirmwareFamily::V1),
        Some("2") => Some(FirmwareFamily::V2),
        _ => None,
    }
}

fn check_firmware(firmware_main: &str, expected: FirmwareFamily) -> Result<(), CryptoError> {
    if firmware_family(firmware_main) == Some(expected) {
        Ok(())
    } else {
        Err(CryptoError::UnsupportedFirmware(firmware_main.to_string()))
    }
}

/// Décode le champ de 24 octets du numéro de série de `.mdf` (offset [`MDF_SERIAL_OFFSET`]) :
/// exactement 14 caractères ASCII hexadécimaux (majuscules ou minuscules), suivis de 10 octets
/// nuls. Le résultat fait 7 octets.
///
/// Tout autre contenu (moins ou plus de 14 caractères, caractère non hexadécimal, nul au
/// milieu, octet non nul dans le bourrage) est refusé : un numéro d'une autre longueur
/// donnerait un fichier `key` qui ne fait pas 32 octets (à confirmer sur appareil, #236).
pub fn parse_serial_field(field: &[u8]) -> Result<[u8; SERIAL_LEN], CryptoError> {
    if field.len() != SERIAL_FIELD_LEN {
        return Err(CryptoError::MalformedSerialField(
            "champ de taille inattendue",
        ));
    }
    let (digits, padding) = field.split_at(2 * SERIAL_LEN);
    if padding.iter().any(|&b| b != 0) {
        return Err(CryptoError::MalformedSerialField(
            "plus de 14 caractères ou bourrage non nul",
        ));
    }
    let nibble = |c: u8| -> Result<u8, CryptoError> {
        match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            0 => Err(CryptoError::MalformedSerialField("moins de 14 caractères")),
            _ => Err(CryptoError::MalformedSerialField(
                "caractère non hexadécimal",
            )),
        }
    };
    let mut serial = [0u8; SERIAL_LEN];
    for (byte, &[hi, lo]) in serial.iter_mut().zip(digits.as_chunks::<2>().0) {
        *byte = (nibble(hi)? << 4) | nibble(lo)?;
    }
    Ok(serial)
}

/// Clés de chiffrement des histoires d'une Flam : en firmware 2.x, lues dans son fichier `.mdf`
/// (disposition dans la documentation du module) ; en firmware 1.x, clés « carrier » d'une
/// histoire officielle du même compte.
///
/// Interface minimale remplie par la détection (#225, #241) : version « main », numéro de
/// série décodé par [`parse_serial_field`] (7 octets), clé à [`MDF_KEY_OFFSET`] et IV à
/// [`MDF_IV_OFFSET`].
pub struct DeviceKeys {
    key_file: Zeroizing<[u8; KEY_FILE_LEN]>,
    story_key: Zeroizing<[u8; 16]>,
    story_iv: Zeroizing<[u8; 16]>,
}

impl DeviceKeys {
    /// Construit les clés d'un appareil. `firmware_main` est la version principale
    /// (« 2.4.2 ») ; `serial` le numéro de série binaire (hexadécimal de `.mdf` décodé).
    ///
    /// Erreur [`CryptoError::UnsupportedFirmware`] hors firmware 2.x (1.x, 3.x et suivants) :
    /// aucune préparation n'est alors possible.
    pub fn new(
        firmware_main: &str,
        serial: &[u8],
        story_key: [u8; 16],
        story_iv: [u8; 16],
    ) -> Result<Self, CryptoError> {
        let story_key = Zeroizing::new(story_key);
        let story_iv = Zeroizing::new(story_iv);
        check_firmware(firmware_main, FirmwareFamily::V2)?;
        let serial: [u8; SERIAL_LEN] = serial
            .try_into()
            .map_err(|_| CryptoError::InvalidSerial(serial.len()))?;
        Ok(Self {
            key_file: key_file_from_serial(&serial),
            story_key,
            story_iv,
        })
    }

    /// Clés d'une Flam en firmware 1.x : celles d'une histoire officielle du même compte
    /// (« carrier », #234), vérifiées au préalable par
    /// [`super::carrier::check_against_device`]. Le fichier `key` produit est celui de
    /// l'histoire officielle.
    ///
    /// Erreur [`CryptoError::UnsupportedFirmware`] hors firmware 1.x : en 2.x, les clés de
    /// `.mdf` restent la seule source.
    pub fn with_carrier(firmware_main: &str, carrier: &CarrierKeys) -> Result<Self, CryptoError> {
        check_firmware(firmware_main, FirmwareFamily::V1)?;
        Ok(Self {
            key_file: Zeroizing::new(*carrier.key_file()),
            story_key: Zeroizing::new(*carrier.story_key()),
            story_iv: Zeroizing::new(*carrier.story_iv()),
        })
    }

    /// Chiffre un script ou le fichier `info` d'une histoire pour cet appareil.
    pub fn encrypt(&self, plain: &[u8]) -> Vec<u8> {
        encrypt_zero_padded(&self.story_key, &self.story_iv, plain)
    }

    /// Titre (première ligne non vide) du fichier `info` chiffré `info` d'une histoire, s'il se
    /// déchiffre avec ces clés en texte lisible (même contrôle que le mode carrier :
    /// [`super::carrier::info_text`]). `None` sinon : la clé ne correspond pas, ou `info` est
    /// abîmé. Le clair est effacé de la mémoire.
    pub fn title_of_info(&self, info: &[u8]) -> Option<String> {
        let text = zeroize::Zeroizing::new(super::carrier::info_text(
            &self.story_key,
            &self.story_iv,
            info,
        )?);
        // Des titres officiels alignent leurs parties avec de longues suites d'espaces
        // (observé sur une Flam 1.x) : elles sont réduites à une seule.
        let title = super::carrier::title_of(&text)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        Some(title).filter(|t| !t.is_empty())
    }

    /// Contenu du fichier `key` d'une histoire (32 octets). Firmware 2.x : déduit du numéro de
    /// série ([`key_file_from_serial`]) ; firmware 1.x : celui de l'histoire officielle.
    pub fn key_file(&self) -> Zeroizing<[u8; KEY_FILE_LEN]> {
        self.key_file.clone()
    }
}

/// Fichier `key` d'une histoire en firmware 2.x (32 octets = 14 + 10 + 8) : le numéro de série
/// en hexadécimal ASCII **minuscule** (14 caractères, quelle que soit la casse dans `.mdf`),
/// 10 octets nuls, puis les 8 premiers caractères de ce même hexadécimal.
pub fn key_file_from_serial(serial: &[u8; SERIAL_LEN]) -> Zeroizing<[u8; KEY_FILE_LEN]> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex = Zeroizing::new([0u8; 2 * SERIAL_LEN]);
    for (i, b) in serial.iter().enumerate() {
        hex[2 * i] = HEX[usize::from(b >> 4)];
        hex[2 * i + 1] = HEX[usize::from(b & 0x0f)];
    }
    let mut out = Zeroizing::new([0u8; KEY_FILE_LEN]);
    out[..hex.len()].copy_from_slice(&hex[..]);
    let tail = hex.len() + KEY_FILE_GAP;
    out[tail..].copy_from_slice(&hex[..KEY_FILE_TAIL]);
    out
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

    /// Champ `.mdf` du numéro de série construit selon la règle : `digits` puis des nuls.
    fn serial_field(digits: &[u8]) -> Vec<u8> {
        let mut field = digits.to_vec();
        field.resize(SERIAL_FIELD_LEN, 0);
        field
    }

    #[test]
    fn disposition_de_mdf_coherente() {
        // 2 octets d'en-tête + 48 de texte ; 24 de numéro ; VID et PID ; clé ; IV ; 16 restants.
        assert_eq!(MDF_SERIAL_OFFSET, 2 + 48);
        assert_eq!(MDF_KEY_OFFSET, MDF_SERIAL_OFFSET + SERIAL_FIELD_LEN + 2 + 2);
        assert_eq!(MDF_IV_OFFSET, MDF_KEY_OFFSET + 16);
        assert_eq!(MDF_LEN - (MDF_IV_OFFSET + 16), 16);
        assert_eq!(2 * SERIAL_LEN + 10 + 8, KEY_FILE_LEN);
    }

    #[test]
    fn champ_numero_de_serie_decode() {
        // Vecteurs inventés, en minuscules, majuscules et casse mêlée.
        for digits in [&b"00123456789abc"[..], b"00123456789ABC", b"00123456789aBc"] {
            assert_eq!(parse_serial_field(&serial_field(digits)), Ok(TEST_SERIAL));
        }
        assert_eq!(
            parse_serial_field(&serial_field(b"fedcba98765432")),
            Ok([0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32])
        );
    }

    #[test]
    fn fichier_key_depuis_un_mdf_en_majuscules() {
        // Champ inventé en majuscules : le fichier `key` reste en minuscules.
        let serial = parse_serial_field(&serial_field(b"3A0B1C2D3E4F50")).unwrap();
        let keys = DeviceKeys::new("2.4.2", &serial, TEST_KEY, TEST_IV).unwrap();
        let mut expected = b"3a0b1c2d3e4f50".to_vec();
        expected.extend_from_slice(&[0u8; 10]);
        expected.extend_from_slice(b"3a0b1c2d");
        assert_eq!(&keys.key_file()[..], &expected[..]);
    }

    #[test]
    fn champ_numero_de_serie_mal_forme_refuse() {
        let mut nul_inside = serial_field(b"00123456789abc");
        nul_inside[5] = 0;
        let mut trailing = serial_field(b"00123456789abc");
        trailing[23] = b'1';
        let mut nul_first = serial_field(b"00123456789abc");
        nul_first[0] = 0;
        let mut nul_last = serial_field(b"00123456789abc");
        nul_last[13] = 0;
        let cases: [Vec<u8>; 12] = [
            nul_first,
            nul_last,
            serial_field("00123456789aé".as_bytes()),
            serial_field(b""),
            serial_field(b"0012345678"),
            serial_field(b"00123456789ab"),
            serial_field(b"00123456789abcd"),
            serial_field(b"00123456789abc0011223344"),
            serial_field(b"00123456789abg"),
            nul_inside,
            trailing,
            b"00123456789abc".to_vec(),
        ];
        for field in cases {
            assert!(
                matches!(
                    parse_serial_field(&field),
                    Err(CryptoError::MalformedSerialField(_))
                ),
                "{field:?}"
            );
        }
        // Le message ne reprend pas le contenu du champ.
        let err = parse_serial_field(&serial_field(b"00123456789abg")).unwrap_err();
        assert!(!err.to_string().contains("0012345"), "{err}");
    }

    #[test]
    fn firmware_hors_2x_refuse() {
        for fw in [
            "1.15.14", "3.0.0", "3.1", "4.0.0", "", "abc", "20.1", "+2.4.2", "02.4", " 2.4.2",
        ] {
            assert_eq!(
                DeviceKeys::new(fw, &TEST_SERIAL, TEST_KEY, TEST_IV).unwrap_err(),
                CryptoError::UnsupportedFirmware(fw.to_string())
            );
        }
        for fw in ["2", "2.0.0", "2.4.2"] {
            assert!(
                DeviceKeys::new(fw, &TEST_SERIAL, TEST_KEY, TEST_IV).is_ok(),
                "{fw}"
            );
        }
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
