//! Image d'une catégorie choisie par le parent (N5 c) : choix appliqué, repli sur l'image
//! automatique avec avertissement, aucun avertissement sans choix.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::build_menu;
use crate::install::pack_speech::SpokenTitles;
use crate::podcast::archive::{MenuEntry, PackEpisode};
use crate::podcast::pack::{EpisodeRef, PackNode};

fn r(guid: &str) -> EpisodeRef {
    EpisodeRef {
        source_id: "s".into(),
        episode_id: guid.into(),
    }
}

fn episode(n: usize, image: bool) -> PackEpisode {
    PackEpisode {
        title: format!("Épisode {n}"),
        audio: PathBuf::from(format!("ep-{n:04}.mp3")),
        image: image.then(|| PathBuf::from(format!("ep-{n:04}.png"))),
        title_audio: None,
    }
}

fn category(entries: Vec<PackNode>, image: Option<EpisodeRef>) -> PackNode {
    PackNode::Category {
        name: "Saison".into(),
        entries,
        image,
    }
}

/// Menu `g0`, puis la catégorie « Saison » (`g1`, `g2`) ; rangs = ordre des guids.
/// `kept` : rangs des épisodes téléchargés ; `no_image` : rangs sans image.
fn run(
    image: Option<EpisodeRef>,
    kept: &[usize],
    no_image: &[usize],
) -> (Vec<MenuEntry>, Vec<String>) {
    let refs = [r("g0"), r("g1"), r("g2")];
    let menu = vec![
        PackNode::Episode(refs[0].clone()),
        category(
            vec![
                PackNode::Episode(refs[1].clone()),
                PackNode::Episode(refs[2].clone()),
            ],
            image,
        ),
    ];
    let ranks: HashMap<&EpisodeRef, usize> = refs.iter().enumerate().map(|(n, e)| (e, n)).collect();
    let mut episodes: HashMap<usize, PackEpisode> = kept
        .iter()
        .map(|&n| (n, episode(n, !no_image.contains(&n))))
        .collect();
    let mut warnings = Vec::new();
    let out = build_menu(
        &menu,
        &ranks,
        &mut episodes,
        &SpokenTitles::default(),
        &mut Vec::new(),
        &mut HashSet::new(),
        &mut warnings,
    );
    (out, warnings)
}

fn category_image(menu: &[MenuEntry]) -> Option<PathBuf> {
    menu.iter().find_map(|e| match e {
        MenuEntry::Category { image, .. } => image.clone(),
        _ => None,
    })
}

#[test]
fn le_choix_est_applique_a_la_categorie() {
    let (menu, warnings) = run(Some(r("g2")), &[0, 1, 2], &[]);
    assert_eq!(category_image(&menu), Some(PathBuf::from("ep-0002.png")));
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn sans_choix_image_automatique_et_aucun_avertissement() {
    let (menu, warnings) = run(None, &[0, 1, 2], &[]);
    assert_eq!(category_image(&menu), None);
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn episode_choisi_retire_du_pack_repli_et_avertissement() {
    // g2 n'a pas été téléchargé (retiré, P7).
    let (menu, warnings) = run(Some(r("g2")), &[0, 1], &[]);
    assert_eq!(category_image(&menu), None);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("Saison"), "{warnings:?}");
    assert!(!warnings[0].contains("://"));
}

#[test]
fn episode_choisi_sans_image_repli_et_avertissement() {
    let (menu, warnings) = run(Some(r("g1")), &[0, 1, 2], &[1]);
    assert_eq!(category_image(&menu), None);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
}

#[test]
fn episode_choisi_hors_de_la_categorie_ou_inconnu_repli_et_avertissement() {
    // g0 est dans le menu racine, pas dans la catégorie ; « zz » n'est dans aucun pack.
    for chosen in [r("g0"), r("zz")] {
        let (menu, warnings) = run(Some(chosen), &[0, 1, 2], &[]);
        assert_eq!(category_image(&menu), None);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }
}

#[test]
fn categorie_videe_n_avertit_pas() {
    // Aucun épisode de la catégorie téléchargé : catégorie retirée (P7), rien à dire.
    let (menu, warnings) = run(Some(r("g1")), &[0], &[]);
    assert_eq!(menu.len(), 1);
    assert!(warnings.is_empty(), "{warnings:?}");
}
