//! Aucune clé d'appareil dans les journaux de la préparation d'une histoire pour la Flam (#228).
//!
//! Test isolé dans son propre binaire : un abonné `tracing` local à un fil manque des
//! événements quand d'autres tests touchent le même point de journalisation en parallèle.

use std::fs::File;
use std::io::{self, Write};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use flam_launcher_lib::device::crypto::DeviceKeys;
use flam_launcher_lib::device::prepare::{prepare_story, Limits};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// Clés inventées (aucune vraie clé d'appareil).
const KEY: [u8; 16] = [0xc3; 16];
const IV: [u8; 16] = [0x5a; 16];
const SERIAL: [u8; 7] = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd];

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn write_pk(path: &std::path::Path, extra: Option<&str>) {
    let mut zip = ZipWriter::new(File::create(path).unwrap());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let mut entries: Vec<(&str, &[u8])> = vec![
        ("uuid.bin", &[0x11; 16]),
        ("info.plain", b"Titre\n"),
        ("version", b"1"),
        ("main.lua", b"function setup() end"),
    ];
    if let Some(name) = extra {
        entries.push((name, b"x"));
    }
    for (name, data) in entries {
        zip.start_file(name, options).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn aucune_cle_dans_les_journaux() {
    let capture = Capture::default();
    let writer = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let good = tmp.path().join("ok.plain.pk");
    let bad = tmp.path().join("ko.plain.pk");
    write_pk(&good, None);
    write_pk(&bad, Some("../evil"));
    let keys = DeviceKeys::new("2.4.2", &SERIAL, &KEY, &IV).unwrap();
    let work = tmp.path().join("travail");
    let cancel = AtomicBool::new(false);
    prepare_story(&good, &keys, &work, &Limits::default(), &cancel).unwrap();
    prepare_story(&bad, &keys, &work, &Limits::default(), &cancel).unwrap_err();

    let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("histoire préparée"), "{logs}");
    for secret in [&KEY[..], &IV[..], &SERIAL[..]] {
        let as_hex: String = secret.iter().map(|b| format!("{b:02x}")).collect();
        assert!(!logs.to_lowercase().contains(&as_hex), "{logs}");
        assert!(!logs.contains(&format!("{secret:?}")), "{logs}");
    }
}
