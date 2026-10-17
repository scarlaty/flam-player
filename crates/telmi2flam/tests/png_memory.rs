//! Bornes de ressources du décodage PNG sur des entrées hostiles : mémoire engagée par une
//! taille annoncée sans données, coût et annulation des chunks de texte.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use telmi2flam::png::{decode_rgba, PngError};

/// Allocateur qui mesure le pic d'octets alloués.
struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
/// Les tests de ce fichier mesurent le pic l'un après l'autre.
static SERIAL: Mutex<()> = Mutex::new(());

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            PEAK.fetch_max(now, Ordering::SeqCst);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            PEAK.fetch_max(now, Ordering::SeqCst);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        CURRENT.fetch_sub(layout.size(), Ordering::SeqCst);
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(data);
    let mut v = (data.len() as u32).to_be_bytes().to_vec();
    v.extend_from_slice(kind);
    v.extend_from_slice(data);
    v.extend_from_slice(&crc.finalize().to_be_bytes());
    v
}

fn png(w: u32, h: u32, depth: u8, color: u8, middle: &[Vec<u8>], idat: &[u8]) -> Vec<u8> {
    let mut ihdr = w.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[depth, color, 0, 0, 0]);
    let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
    v.extend(chunk(b"IHDR", &ihdr));
    for c in middle {
        v.extend_from_slice(c);
    }
    v.extend(chunk(b"IDAT", idat));
    v.extend(chunk(b"IEND", &[]));
    v
}

fn never() -> bool {
    false
}

#[test]
fn taille_annoncee_sans_donnees_n_engage_pas_de_memoire() {
    let _serial = SERIAL.lock().unwrap();
    // 178 956 970 x 1 en RGBA 16 bits : 1,4 Go de lignes brutes et 716 Mo de RGBA annoncés
    let idat = miniz_oxide::deflate::compress_to_vec_zlib(&[0u8; 100], 6);
    let data = png(178_956_970, 1, 16, 6, &[], &idat);
    assert!(data.len() < 100);
    let base = CURRENT.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let result = decode_rgba(&data, &never);
    let peak = PEAK.load(Ordering::SeqCst) - base;
    assert!(matches!(result, Err(PngError::Unreadable(_))), "{result:?}");
    assert!(peak < 64 << 20, "pic de {peak} octets");
}

/// `n` chunks zTXt (petit texte compressé) avant l'IDAT d'une image 1 x 1.
fn many_ztxt(n: usize) -> Vec<u8> {
    let body = {
        let mut b = b"k\x00\x00".to_vec();
        b.extend(miniz_oxide::deflate::compress_to_vec_zlib(b"v", 6));
        chunk(b"zTXt", &b)
    };
    let idat = miniz_oxide::deflate::compress_to_vec_zlib(&[0, 0], 6);
    png(1, 1, 8, 0, &vec![body; n], &idat)
}

#[test]
fn cent_mille_chunks_ztxt_restent_rapides() {
    let _serial = SERIAL.lock().unwrap();
    let data = many_ztxt(100_000);
    let start = Instant::now();
    let img = decode_rgba(&data, &never).unwrap();
    let elapsed = start.elapsed();
    assert_eq!((img.width, img.height), (1, 1));
    // ~1 s en debug sur un poste chargé ; bien moins en release
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
}

#[test]
fn annulation_pendant_les_chunks_avant_l_image() {
    let _serial = SERIAL.lock().unwrap();
    let data = many_ztxt(1000);
    let calls = Cell::new(0);
    let stop = || {
        calls.set(calls.get() + 1);
        calls.get() > 10
    };
    assert_eq!(decode_rgba(&data, &stop), Err(PngError::Cancelled));
    assert_eq!(calls.get(), 11);
}

#[test]
fn annulation_pendant_les_chunks_apres_l_image() {
    let _serial = SERIAL.lock().unwrap();
    // image 1 x 1 (une ligne), puis 1000 chunks de texte après l'IDAT
    let mut data = many_ztxt(0);
    data.truncate(data.len() - 12); // retire l'IEND
    let text = chunk(b"tEXt", b"k\x00v");
    for _ in 0..1000 {
        data.extend_from_slice(&text);
    }
    let calls = Cell::new(0);
    let stop = || {
        calls.set(calls.get() + 1);
        calls.get() > 20
    };
    assert_eq!(decode_rgba(&data, &stop), Err(PngError::Cancelled));
}
