//! Décompresseurs incrémentaux des méthodes ZIP bzip2 (12), LZMA (14) et Zstandard (93), lues
//! par le `zipfile` de Python 3.14 (#458). Chaque décodeur reproduit l'objet `decompress()` que
//! `zipfile` utilise (`bz2.BZ2Decompressor`, `zipfile.LZMADecompressor`,
//! `compression.zstd.ZstdDecompressor`) : il reçoit les octets compressés par morceaux, garde ce
//! qu'il ne peut pas encore décoder, rend la sortie au fil de l'eau et signale la fin du flux
//! (`eof`). Un flux tronqué n'est pas une erreur : la sortie s'arrête là où la bibliothèque C
//! s'arrêterait, et c'est le contrôle du CRC de l'entrée qui décide (comme en Python).
//!
//! Les écarts connus avec les bibliothèques C (tous sur des flux corrompus ou tronqués, jamais
//! sur un flux valide) sont listés dans le tableau des pannes de `PORT-RUST.md` (§3).

/// Arrêt d'un décodeur.
#[derive(Debug, PartialEq, Eq)]
pub enum Halt {
    /// Flux invalide : `OSError` (bz2), `LZMAError` ou `ZstdError` en Python (code 6), code 3
    /// en Rust (« Python échoue aussi », comme pour une entrée chiffrée).
    Data(String),
    /// Le destinataire de la sortie a refusé un morceau : arrêt sans autre message.
    Sink,
}

pub(crate) fn data(message: impl Into<String>) -> Halt {
    Halt::Data(message.into())
}

/// Destinataire de la sortie : `Err(())` arrête le décodage ([`Halt::Sink`]).
pub type Out<'a> = dyn FnMut(&[u8]) -> Result<(), ()> + 'a;

/// Décompresseur incrémental.
pub trait StreamDecoder {
    /// Ajoute `input` et décode ce qui peut l'être ; chaque morceau décodé est passé à `out`.
    /// `flush` : décoder tout ce qui est possible maintenant (fin d'un tour de lecture) ; sans
    /// `flush`, le décodeur peut attendre la suite (seulement pour éviter des tentatives de
    /// décodage répétées sur un bloc incomplet : la sortie finale n'en dépend pas).
    /// Rend `true` quand la fin du flux est atteinte (`decompressor.eof`) ; les octets reçus
    /// ensuite sont ignorés (`unused_data`).
    fn feed(&mut self, input: &[u8], flush: bool, out: &mut Out) -> Result<bool, Halt>;

    /// Plus aucun octet n'arrivera et la fin du flux n'est pas atteinte (flux tronqué) : rend
    /// ce que la bibliothèque C aurait déjà rendu et que le décodeur retient encore.
    fn finish(&mut self, _out: &mut Out) -> Result<(), Halt> {
        Ok(())
    }
}

/// Sortie tamponnée par morceaux d'au plus [`OUT_CHUNK`] octets.
pub(crate) struct OutBuf {
    buf: Vec<u8>,
}

/// Taille des morceaux de sortie passés à `out`.
pub(crate) const OUT_CHUNK: usize = 64 * 1024;

impl OutBuf {
    pub(crate) fn new() -> Self {
        OutBuf {
            buf: Vec::with_capacity(OUT_CHUNK),
        }
    }

    #[inline]
    pub(crate) fn push(&mut self, b: u8, out: &mut Out) -> Result<(), Halt> {
        self.buf.push(b);
        if self.buf.len() >= OUT_CHUNK {
            self.flush(out)?;
        }
        Ok(())
    }

    pub(crate) fn flush(&mut self, out: &mut Out) -> Result<(), Halt> {
        if !self.buf.is_empty() {
            let r = out(&self.buf);
            self.buf.clear();
            r.map_err(|()| Halt::Sink)?;
        }
        Ok(())
    }
}
