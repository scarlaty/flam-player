//! Tests de `parse_feed` sur des flux synthétiques (aucun flux réel versionné).

use super::*;

fn wrap(channel: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd"
     xmlns:dc="http://purl.org/dc/elements/1.1/">
<channel>{channel}</channel></rss>"#
    )
}

fn item(guid: &str, extra: &str) -> String {
    format!(
        r#"<item><title>Ep {guid}</title><guid>{guid}</guid>
        <enclosure url="https://cdn.example/{guid}.mp3" type="audio/mpeg" length="1234"/>{extra}</item>"#
    )
}

fn parse(xml: &str) -> Result<PodcastFeed, RssError> {
    parse_feed(xml.as_bytes())
}

#[test]
fn flux_complet() {
    let xml = wrap(
        r#"<title>  Mon
        podcast </title>
        <link>https://exemple.fr/</link>
        <description><![CDATA[<p>Des <b>histoires</b> &amp; contes.</p><p>Deux.</p>]]></description>
        <copyright>(c) Moi</copyright>
        <image><url>https://exemple.fr/cover.png</url><title>x</title></image>
        <itunes:image href="https://exemple.fr/itunes.jpg"/>
        <item>
          <title>Épisode un</title>
          <guid isPermaLink="false">abc-1</guid>
          <pubDate>Tue, 03 Jun 2008 11:05:30 GMT</pubDate>
          <itunes:duration>01:02:03</itunes:duration>
          <itunes:author>Alice</itunes:author>
          <itunes:image href="https://exemple.fr/ep1.jpg"/>
          <description>Résumé &lt;i&gt;court&lt;/i&gt;</description>
          <enclosure url="https://cdn.exemple.fr/1.mp3" length="4242" type="audio/mpeg"/>
        </item>"#,
    );
    let feed = parse(&xml).unwrap();
    assert_eq!(feed.title, "Mon podcast");
    assert_eq!(feed.link.as_deref(), Some("https://exemple.fr/"));
    assert_eq!(
        feed.description.as_deref(),
        Some("Des histoires & contes.\n\nDeux.")
    );
    assert_eq!(feed.copyright.as_deref(), Some("(c) Moi"));
    assert_eq!(feed.image.as_deref(), Some("https://exemple.fr/cover.png"));
    assert_eq!(feed.episodes.len(), 1);
    let ep = &feed.episodes[0];
    assert_eq!(ep.guid, "abc-1");
    assert_eq!(ep.title, "Épisode un");
    assert_eq!(ep.description.as_deref(), Some("Résumé court"));
    assert_eq!(ep.pub_date.as_deref(), Some("2008-06-03T11:05:30Z"));
    assert_eq!(ep.duration, Some(3723));
    assert_eq!(ep.author.as_deref(), Some("Alice"));
    assert_eq!(ep.image.as_deref(), Some("https://exemple.fr/ep1.jpg"));
    assert_eq!(ep.enclosure_url, "https://cdn.exemple.fr/1.mp3");
    assert_eq!(ep.enclosure_type, "audio/mpeg");
    assert_eq!(ep.enclosure_length, Some(4242));
    assert!(feed.warnings.is_empty());
}

#[test]
fn sans_image_de_canal_pas_de_rejet() {
    let feed = parse(&wrap(&format!("<title>T</title>{}", item("a", "")))).unwrap();
    assert_eq!(feed.image, None);
    assert_eq!(feed.episodes[0].image, None);
}

#[test]
fn repli_des_images() {
    // Pas de balise image : image iTunes du canal ; l'épisode sans image en hérite.
    let xml = wrap(&format!(
        r#"<itunes:image href="https://x.fr/c.jpg"/>{}{}"#,
        item("a", ""),
        item("b", r#"<itunes:image href="https://x.fr/b.jpg"/>"#)
    ));
    let feed = parse(&xml).unwrap();
    assert_eq!(feed.image.as_deref(), Some("https://x.fr/c.jpg"));
    assert_eq!(
        feed.episodes[0].image.as_deref(),
        Some("https://x.fr/c.jpg")
    );
    assert_eq!(
        feed.episodes[1].image.as_deref(),
        Some("https://x.fr/b.jpg")
    );
    // Ni image de canal ni iTunes : image du premier épisode qui en a une.
    let xml = wrap(&format!(
        "{}{}",
        item("a", ""),
        item("b", r#"<itunes:image href="https://x.fr/b.jpg"/>"#)
    ));
    let feed = parse(&xml).unwrap();
    assert_eq!(feed.image.as_deref(), Some("https://x.fr/b.jpg"));
    assert_eq!(
        feed.episodes[0].image.as_deref(),
        Some("https://x.fr/b.jpg")
    );
}

#[test]
fn images_non_web_ignorees() {
    let xml = wrap(&format!(
        r#"<itunes:image href="file:///etc/passwd"/><image><url>javascript:alert(1)</url></image>{}"#,
        item("a", r#"<itunes:image href="ftp://h/x.png"/>"#)
    ));
    let feed = parse(&xml).unwrap();
    assert_eq!(feed.image, None);
    assert_eq!(feed.episodes[0].image, None);
}

#[test]
fn enclosures_multiples_premiere_audio() {
    let xml = wrap(
        r#"<item><title>T</title><guid>g</guid>
        <enclosure url="https://h/cover.jpg" type="image/jpeg" length="1"/>
        <enclosure url="https://h/a.ogg" type="Audio/OGG" length="2"/>
        <enclosure url="https://h/b.mp3" type="audio/mpeg" length="3"/></item>"#,
    );
    let ep = &parse(&xml).unwrap().episodes[0];
    assert_eq!(ep.enclosure_url, "https://h/a.ogg");
    assert_eq!(ep.enclosure_type, "audio/ogg");
    assert_eq!(ep.enclosure_length, Some(2));
}

#[test]
fn episodes_sans_audio_ou_bande_annonce_ignores() {
    let xml = wrap(&format!(
        r#"{}
        <item><title>Vidéo</title><enclosure url="https://h/v.mp4" type="video/mp4"/></item>
        <item><title>Sans</title></item>
        <item><title>Url relative</title><enclosure url="a.mp3" type="audio/mpeg"/></item>
        <item><title>Url file</title><enclosure url="file:///c/a.mp3" type="audio/mpeg"/></item>
        {}"#,
        item(
            "trailer",
            "<itunes:episodeType>Trailer</itunes:episodeType>"
        ),
        item("full", "<itunes:episodeType>full</itunes:episodeType>"),
    ));
    let feed = parse(&xml).unwrap();
    assert_eq!(feed.episodes.len(), 1);
    assert_eq!(feed.episodes[0].guid, "full");
    assert_eq!(feed.warnings.len(), 5);
    assert!(feed.warnings[0].contains("bande-annonce"));
}

#[test]
fn aucun_episode_audio() {
    let xml = wrap(r#"<title>T</title><item><title>x</title></item>"#);
    assert_eq!(parse(&xml), Err(RssError::NoAudioEpisode));
}

#[test]
fn date_absente_ou_illisible_donne_none() {
    let xml = wrap(&format!(
        "{}{}",
        item("a", ""),
        item("b", "<pubDate>n'importe quoi</pubDate>")
    ));
    let feed = parse(&xml).unwrap();
    assert_eq!(feed.episodes[0].pub_date, None);
    assert_eq!(feed.episodes[1].pub_date, None);
}

#[test]
fn dates_variees() {
    let cases = [
        ("Tue, 03 Jun 2008 11:05:30 GMT", "2008-06-03T11:05:30Z"),
        ("3 Jun 2008 11:05 GMT", "2008-06-03T11:05:00Z"),
        ("Tue, 03 Jun 08 11:05:30 +0200", "2008-06-03T09:05:30Z"),
        ("Mon, 31 Dec 1999 23:30:00 -0100", "2000-01-01T00:30:00Z"),
        ("Wed, 01 Mar 2000 00:00:00 EST", "2000-03-01T05:00:00Z"),
        ("Sat, 29 Feb 2020 12:00:00 PST", "2020-02-29T20:00:00Z"),
        ("Fri, 03 Jun 2011 11:05:30", "2011-06-03T11:05:30Z"),
        ("03 JUN 2011 11:05:30 UT", "2011-06-03T11:05:30Z"),
        ("Tue, 03 Jun 2008 11:05:30 +02:00", "2008-06-03T09:05:30Z"),
        ("2021-03-04T05:06:07Z", "2021-03-04T05:06:07Z"),
        ("2021-03-04T05:06:07+01:00", "2021-03-04T04:06:07Z"),
        ("2021-03-04", "2021-03-04T00:00:00Z"),
    ];
    for (raw, expected) in cases {
        assert_eq!(parse_pub_date(raw).as_deref(), Some(expected), "{raw}");
    }
    for bad in [
        "",
        "hier",
        "31 Feb 2020 10:00:00 GMT",
        "01 Jan 2020 25:00:00",
        "Tue, 03 Foo 2008 11:05:30",
    ] {
        assert_eq!(parse_pub_date(bad), None, "{bad:?}");
    }
}

#[test]
fn durees() {
    assert_eq!(parse_duration("3600"), Some(3600));
    assert_eq!(parse_duration("12:34"), Some(754));
    assert_eq!(parse_duration("1:02:03"), Some(3723));
    assert_eq!(parse_duration("90.5"), Some(90));
    assert_eq!(parse_duration("0"), None);
    assert_eq!(parse_duration("abc"), None);
    assert_eq!(parse_duration("1:2:3:4"), None);
    assert_eq!(parse_duration("99999999999999999999"), None);
}

#[test]
fn html_retire_des_descriptions() {
    assert_eq!(
        strip_html(
            "<p>Un <a href=\"x\">lien</a><br/>suite</p><script>alert(1)</script><style>p{}</style>fin"
        ),
        "Un lien\nsuite\nfin"
    );
    // Entités décodées une seule fois, jamais ré-interprétées comme balises.
    assert_eq!(
        strip_html("&lt;b&gt;gras&lt;/b&gt; &amp;amp; &#233;&#x41;"),
        "<b>gras</b> &amp; éA"
    );
    assert_eq!(strip_html("1 < 2 et 3 > 2"), "1 < 2 et 3 > 2");
    assert_eq!(strip_html("a<!-- caché -->b"), "ab");
    assert_eq!(strip_html("a<b"), "a<b");
    assert_eq!(strip_html("a &inconnue; &"), "a &inconnue; &");
}

#[test]
fn description_depuis_summary_en_repli() {
    let xml = wrap(&format!(
        "<itunes:summary>Canal <b>résumé</b></itunes:summary>{}",
        item("a", "<itunes:summary>Épisode résumé</itunes:summary>")
    ));
    let feed = parse(&xml).unwrap();
    assert_eq!(feed.description.as_deref(), Some("Canal résumé"));
    assert_eq!(
        feed.episodes[0].description.as_deref(),
        Some("Épisode résumé")
    );
}

#[test]
fn ordre_des_balises_et_champs_manquants() {
    // Éléments du canal après les épisodes, épisode sans titre ni guid.
    let xml = wrap(
        r#"<item><enclosure url="https://h/a.mp3" type="audio/mpeg"/></item>
        <title>Tardif</title>
        <copyright>© Eux</copyright>"#,
    );
    let feed = parse(&xml).unwrap();
    assert_eq!(feed.title, "Tardif");
    let ep = &feed.episodes[0];
    assert_eq!(ep.guid, "https://h/a.mp3");
    assert_eq!(ep.title, "Épisode 1");
    assert_eq!(ep.author.as_deref(), Some("© Eux"));
    assert_eq!(ep.enclosure_length, None);
}

#[test]
fn titre_de_canal_absent() {
    let feed = parse(&wrap(&item("a", ""))).unwrap();
    assert_eq!(feed.title, "Podcast sans titre");
}

#[test]
fn titre_de_l_image_n_est_pas_le_titre_du_canal() {
    let xml = wrap(&format!(
        "<image><title>Logo</title><url>https://h/l.png</url></image><title>Vrai</title>{}",
        item("a", "")
    ));
    assert_eq!(parse(&xml).unwrap().title, "Vrai");
}

#[test]
fn auteurs_et_doublons() {
    let xml = wrap(&format!(
        "<itunes:author>Canal</itunes:author>{}{}{}{}",
        item("a", "<author>x@y.fr (Bob)</author>"),
        item("b", "<dc:creator>Carl</dc:creator>"),
        item("c", ""),
        item("a", "")
    ));
    let feed = parse(&xml).unwrap();
    let authors: Vec<_> = feed.episodes.iter().map(|e| e.author.as_deref()).collect();
    assert_eq!(authors, [Some("x@y.fr (Bob)"), Some("Carl"), Some("Canal")]);
    assert_eq!(feed.warnings.len(), 1);
    assert!(feed.warnings[0].contains("double"));
}

#[test]
fn bornes_de_longueur() {
    let long = "é".repeat(5_000);
    let xml = wrap(&format!(
        "<title>{long}</title><description>{long}</description>\
         <item><title>{long}</title><enclosure url=\"https://h/a.mp3\" type=\"audio/mpeg\"/></item>"
    ));
    let feed = parse(&xml).unwrap();
    assert_eq!(feed.title.chars().count(), MAX_TITLE_CHARS);
    assert_eq!(
        feed.description.unwrap().chars().count(),
        MAX_DESCRIPTION_CHARS
    );
    assert_eq!(feed.episodes[0].title.chars().count(), MAX_TITLE_CHARS);
    let url = format!("https://h/{}.mp3", "a".repeat(3_000));
    let xml = wrap(&format!(
        r#"<item><title>x</title><enclosure url="{url}" type="audio/mpeg"/></item>"#
    ));
    assert_eq!(parse(&xml), Err(RssError::NoAudioEpisode));
}

#[test]
fn texte_enorme_dans_un_element_est_borne() {
    let huge = "x".repeat(10 * 1024 * 1024);
    let xml = wrap(&format!(
        "<description>{huge}</description>{}",
        item("a", "")
    ));
    let feed = parse(&xml).unwrap();
    assert_eq!(
        feed.description.unwrap().chars().count(),
        MAX_DESCRIPTION_CHARS
    );
}

#[test]
fn nombre_d_episodes_borne() {
    let items: String = (0..MAX_EPISODES + 50)
        .map(|i| item(&i.to_string(), ""))
        .collect();
    let feed = parse(&wrap(&items)).unwrap();
    assert_eq!(feed.episodes.len(), MAX_EPISODES);
    assert_eq!(feed.episodes[0].guid, "0");
    assert!(feed.warnings.iter().any(|w| w.contains("tronqué")));
}

#[test]
fn avertissements_bornes() {
    let items: String = (0..200).map(|_| "<item><title>x</title></item>").collect();
    let xml = wrap(&format!("{items}{}", item("ok", "")));
    let feed = parse(&xml).unwrap();
    assert_eq!(feed.warnings.len(), MAX_WARNINGS + 1);
    assert!(feed.warnings.last().unwrap().starts_with("… et 150"));
}

#[test]
fn flux_trop_grand() {
    let big = vec![b' '; MAX_FEED_BYTES + 1];
    assert_eq!(
        parse_feed(&big),
        Err(RssError::FeedTooLarge {
            size: MAX_FEED_BYTES + 1,
            max: MAX_FEED_BYTES
        })
    );
    // À la limite exacte : pas « trop grand » (mais invalide).
    let edge = vec![b' '; MAX_FEED_BYTES];
    assert!(matches!(parse_feed(&edge), Err(RssError::InvalidFeed(_))));
}

#[test]
fn billion_laughs_refuse() {
    let xml = r#"<?xml version="1.0"?>
<!DOCTYPE rss [
<!ENTITY a "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa">
<!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;">
<!ENTITY c "&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;">
<!ENTITY d "&c;&c;&c;&c;&c;&c;&c;&c;&c;&c;">
]>
<rss version="2.0"><channel><title>&d;</title></channel></rss>"#;
    assert_eq!(parse(xml), Err(RssError::DtdRefused));
}

#[test]
fn dtd_et_entites_externes_refusees() {
    for doctype in [
        r#"<!DOCTYPE rss SYSTEM "http://127.0.0.1:9/evil.dtd">"#,
        r#"<!DOCTYPE rss PUBLIC "-//X//EN" "http://127.0.0.1:9/evil.dtd">"#,
        r#"<!DOCTYPE rss [<!ENTITY xxe SYSTEM "file:///C:/Windows/win.ini">]>"#,
        r#"<!DOCTYPE rss [<!ENTITY % p SYSTEM "http://127.0.0.1:9/p.dtd"> %p;]>"#,
    ] {
        let xml = format!(
            r#"<?xml version="1.0"?>{doctype}<rss version="2.0"><channel><title>&xxe;</title>{}</channel></rss>"#,
            item("a", "")
        );
        assert_eq!(parse(&xml), Err(RssError::DtdRefused), "{doctype}");
    }
}

#[test]
fn entite_non_definie_ignoree_sans_dtd() {
    let xml = wrap(&format!("<title>A&xxe;B&nbsp;C</title>{}", item("a", "")));
    assert_eq!(parse(&xml).unwrap().title, "AB C");
}

#[test]
fn references_numeriques_et_predefinies() {
    let xml = wrap(&format!(
        "<title>&#72;&#x69; &lt;&amp;&gt; &quot;&apos;</title>{}",
        item("a", "")
    ));
    assert_eq!(parse(&xml).unwrap().title, "Hi <&> \"'");
    // Référence invalide (NUL) : ignorée.
    let xml = wrap(&format!("<title>a&#0;b</title>{}", item("a", "")));
    assert_eq!(parse(&xml).unwrap().title, "ab");
}

#[test]
fn imbrication_trop_profonde() {
    let deep = "<a>".repeat(200);
    let xml = wrap(&format!("{deep}{}", item("a", "")));
    let err = parse(&xml).unwrap_err();
    assert!(
        matches!(err, RssError::InvalidFeed(ref m) if m.contains("profonde")),
        "{err}"
    );
}

#[test]
fn flux_invalides() {
    for xml in [
        "",
        "pas du xml",
        "<rss><channel><title>x</title>",
        "<rss><channel></rss>",
        "<rss version=\"2.0\"></rss>",
        "<feed xmlns=\"http://www.w3.org/2005/Atom\"><title>x</title></feed>",
        "<rss><channel/></rss><rss/>",
        "<html><body>404</body></html>",
    ] {
        assert!(
            matches!(parse(xml), Err(RssError::InvalidFeed(_))),
            "{xml:?} : {:?}",
            parse(xml)
        );
    }
}

#[test]
fn messages_d_erreur_distincts() {
    let messages = [
        RssError::FeedTooLarge { size: 2, max: 1 }.to_string(),
        RssError::DtdRefused.to_string(),
        RssError::UnsupportedEncoding("x".into()).to_string(),
        RssError::InvalidFeed("x".into()).to_string(),
        RssError::NoAudioEpisode.to_string(),
    ];
    let unique: HashSet<_> = messages.iter().collect();
    assert_eq!(unique.len(), messages.len());
}

#[test]
fn cdata_et_texte_mixte() {
    let xml = wrap(&format!(
        "<title>A<![CDATA[ <B> & ]]>C</title>{}",
        item("a", "")
    ));
    assert_eq!(parse(&xml).unwrap().title, "A <B> & C");
}

#[test]
fn encodage_iso_8859_1_et_windows_1252() {
    let body = format!(
        "<rss><channel><title>Caf\u{e9} \u{80}</title>{}</channel></rss>",
        item("a", "")
    );
    let to_bytes = |decl: &str| {
        let text = format!("<?xml version=\"1.0\" encoding=\"{decl}\"?>{body}");
        text.chars().map(|c| c as u8).collect::<Vec<u8>>()
    };
    assert_eq!(parse_feed(&to_bytes("ISO-8859-1")).unwrap().title, "Café €");
    assert_eq!(
        parse_feed(&to_bytes("windows-1252")).unwrap().title,
        "Café €"
    );
}

#[test]
fn encodage_utf8_bom_et_utf16() {
    let xml = wrap(&format!("<title>Été</title>{}", item("a", "")));
    let mut bom = vec![0xEF, 0xBB, 0xBF];
    bom.extend_from_slice(xml.as_bytes());
    assert_eq!(parse_feed(&bom).unwrap().title, "Été");

    let units: Vec<u16> = xml.encode_utf16().collect();
    let mut le = vec![0xFF, 0xFE];
    le.extend(units.iter().flat_map(|u| u.to_le_bytes()));
    assert_eq!(parse_feed(&le).unwrap().title, "Été");
    let mut be = vec![0xFE, 0xFF];
    be.extend(units.iter().flat_map(|u| u.to_be_bytes()));
    assert_eq!(parse_feed(&be).unwrap().title, "Été");
    // UTF-16 invalide (substitut isolé).
    assert!(matches!(
        parse_feed(&[0xFF, 0xFE, 0x00, 0xD8, 0x41, 0x00]),
        Err(RssError::InvalidFeed(_))
    ));
}

#[test]
fn encodage_non_pris_en_charge_et_utf8_mal_etiquete() {
    let xml = r#"<?xml version="1.0" encoding="Shift_JIS"?><rss><channel/></rss>"#;
    assert!(matches!(parse(xml), Err(RssError::UnsupportedEncoding(ref e)) if e == "shift_jis"));
    // Déclaré UTF-8 mais contenant un octet isolé : tolérance (caractère de remplacement).
    let mut bytes = wrap(&format!("<title>a_b</title>{}", item("a", ""))).into_bytes();
    let at = bytes.windows(3).position(|w| w == b"a_b").unwrap() + 1;
    bytes[at] = 0xE9;
    assert_eq!(parse_feed(&bytes).unwrap().title, "a\u{FFFD}b");
}

#[test]
fn caracteres_de_controle_retires() {
    let xml = wrap(&format!("<title>a\u{7}b\tc\nd</title>{}", item("a", "")));
    assert_eq!(parse(&xml).unwrap().title, "a b c d");
}

#[test]
fn guid_trop_long_est_borne() {
    let guid = "g".repeat(2_000);
    let feed = parse(&wrap(&item(&guid, ""))).unwrap();
    assert_eq!(feed.episodes[0].guid.chars().count(), MAX_GUID_CHARS);
}

#[test]
fn balises_inconnues_et_casse() {
    let xml = wrap(&format!(
        "<unknown><title>faux</title></unknown><TITLE>Vrai</TITLE>{}",
        item("a", "<foo><bar/></foo>")
    ));
    assert_eq!(parse(&xml).unwrap().title, "Vrai");
}

#[test]
fn script_en_majuscules_et_nombreux_restent_rapides() {
    assert_eq!(strip_html("a<SCRIPT>x</Script>b<STYLE>y</STYLE>c"), "abc");
    let many = "<script>x</script>".repeat(20_000);
    let start = std::time::Instant::now();
    assert_eq!(strip_html(&format!("{many}fin")), "fin");
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
}

/// Flux de ~15 Mio dont le canal porte `body` dans une description CDATA : l'analyse doit
/// rester rapide (bornes de texte et d'entrée HTML).
fn hostile_description(body: &str) -> (PodcastFeed, std::time::Duration) {
    let reps = 15 * 1024 * 1024 / body.len();
    let xml = wrap(&format!(
        "<description><![CDATA[{}]]></description>{}",
        body.repeat(reps),
        item("a", "")
    ));
    let start = std::time::Instant::now();
    let feed = parse(&xml).unwrap();
    (feed, start.elapsed())
}

#[test]
fn cdata_geant_d_esperluettes_reste_rapide() {
    let (feed, took) = hostile_description("&");
    assert!(feed.description.is_some());
    assert!(took < std::time::Duration::from_secs(5), "{took:?}");
}

#[test]
fn cdata_geant_de_balises_script_reste_rapide() {
    let (feed, took) = hostile_description("<script></script>");
    assert!(feed.description.as_deref().unwrap_or("").len() <= 1);
    assert!(took < std::time::Duration::from_secs(5), "{took:?}");
}

#[test]
fn titre_geant_reste_rapide_et_borne() {
    let huge = "mot ".repeat(3 * 1024 * 1024);
    let xml = wrap(&format!(
        "<title><![CDATA[{huge}]]></title>{}",
        item("a", "")
    ));
    let start = std::time::Instant::now();
    let feed = parse(&xml).unwrap();
    assert_eq!(feed.title.chars().count(), MAX_TITLE_CHARS);
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn script_autofermant_ne_masque_pas_la_suite() {
    assert_eq!(strip_html("a<script src=\"x\"/>b"), "ab");
}

#[test]
fn date_iso_avec_espace_avant_le_fuseau() {
    assert_eq!(
        parse_pub_date("2008-06-03 11:05:30 +0200").as_deref(),
        Some("2008-06-03T09:05:30Z")
    );
}

#[test]
fn doctype_en_minuscules_ou_apres_le_contenu_refuse() {
    let late = "<rss><channel><title>x</title></channel><!DOCTYPE rss></rss>";
    assert_eq!(parse(late), Err(RssError::DtdRefused));
    let lower = r#"<!doctype rss [<!entity a "b">]><rss><channel/></rss>"#;
    assert_eq!(parse(lower), Err(RssError::DtdRefused));
}

// --- itunes:explicit (#361) ---

fn explicits(feed: &PodcastFeed) -> Vec<Option<bool>> {
    feed.episodes.iter().map(|e| e.explicit).collect()
}

#[test]
fn explicite_valeur_de_l_episode() {
    let xml = wrap(&format!(
        "<title>T</title>{}{}{}",
        item("a", "<itunes:explicit>yes</itunes:explicit>"),
        item("b", "<itunes:explicit>clean</itunes:explicit>"),
        item("c", "<itunes:explicit>explicit</itunes:explicit>"),
    ));
    let feed = parse(&xml).unwrap();
    assert_eq!(feed.explicit, None);
    assert_eq!(explicits(&feed), [Some(true), Some(false), Some(true)]);
}

#[test]
fn explicite_valeur_du_canal_heritee() {
    let xml = wrap(&format!(
        "<title>T</title><itunes:explicit>true</itunes:explicit>{}{}",
        item("a", ""),
        item("b", "<itunes:explicit>no</itunes:explicit>"),
    ));
    let feed = parse(&xml).unwrap();
    assert_eq!(feed.explicit, Some(true));
    // L'épisode sans valeur hérite du canal ; sa propre valeur prime.
    assert_eq!(explicits(&feed), [Some(true), Some(false)]);
}

#[test]
fn explicite_canal_apres_les_episodes() {
    let xml = wrap(&format!(
        "<title>T</title>{}<itunes:explicit>false</itunes:explicit>",
        item("a", ""),
    ));
    let feed = parse(&xml).unwrap();
    assert_eq!(feed.explicit, Some(false));
    assert_eq!(explicits(&feed), [Some(false)]);
}

#[test]
fn explicite_absent_reste_inconnu() {
    let feed = parse(&wrap(&format!("<title>T</title>{}", item("a", "")))).unwrap();
    assert_eq!(feed.explicit, None);
    assert_eq!(explicits(&feed), [None]);
}

#[test]
fn explicite_valeurs_invalides_ignorees() {
    let xml = wrap(&format!(
        "<title>T</title><itunes:explicit>peut-etre</itunes:explicit>{}{}{}",
        item("a", "<itunes:explicit>1</itunes:explicit>"),
        item("b", "<itunes:explicit></itunes:explicit>"),
        item(
            "c",
            &format!("<itunes:explicit>{}</itunes:explicit>", "x".repeat(10_000))
        ),
    ));
    let feed = parse(&xml).unwrap();
    assert_eq!(feed.explicit, None);
    assert_eq!(explicits(&feed), [None, None, None]);
    // Une valeur invalide d'épisode laisse jouer la valeur du canal.
    let xml = wrap(&format!(
        "<title>T</title><itunes:explicit>yes</itunes:explicit>{}",
        item("a", "<itunes:explicit>bof</itunes:explicit>"),
    ));
    assert_eq!(explicits(&parse(&xml).unwrap()), [Some(true)]);
}

#[test]
fn explicite_casse_et_espaces() {
    let xml = wrap(&format!(
        "<title>T</title>{}{}{}{}",
        item("a", "<itunes:explicit>  YES\n</itunes:explicit>"),
        item("b", "<itunes:explicit> False </itunes:explicit>"),
        item(
            "c",
            "<itunes:explicit><![CDATA[Explicit]]></itunes:explicit>"
        ),
        item("d", "<ITUNES:Explicit>NO</ITUNES:Explicit>"),
    ));
    assert_eq!(
        explicits(&parse(&xml).unwrap()),
        [Some(true), Some(false), Some(true), Some(false)]
    );
}

#[test]
fn explicite_premiere_valeur_valide_gagne() {
    let xml = wrap(&format!(
        "<title>T</title>{}",
        item(
            "a",
            "<itunes:explicit>??</itunes:explicit><itunes:explicit>yes</itunes:explicit>\
             <itunes:explicit>no</itunes:explicit>"
        ),
    ));
    assert_eq!(explicits(&parse(&xml).unwrap()), [Some(true)]);
}

#[test]
fn explicite_flux_type_apple_et_ignore_hors_canal() {
    // Forme courante : valeur au canal, `clean` par épisode ; un `itunes:explicit` imbriqué
    // ailleurs (ici dans `itunes:owner`) ne compte pas.
    let xml = wrap(&format!(
        "<title>T</title><itunes:owner><itunes:explicit>yes</itunes:explicit></itunes:owner>\
         <itunes:explicit>no</itunes:explicit>{}{}",
        item("a", "<itunes:explicit>clean</itunes:explicit>"),
        item("b", "<itunes:explicit>yes</itunes:explicit>"),
    ));
    let feed = parse(&xml).unwrap();
    assert_eq!(feed.explicit, Some(false));
    assert_eq!(explicits(&feed), [Some(false), Some(true)]);
}

// ---------------------------------------------------------------------------------------
// Suivis de #249
// ---------------------------------------------------------------------------------------

#[test]
fn esperluette_et_chevron_isoles_ne_rejettent_pas_le_flux() {
    let xml = wrap(&format!(
        "<title>AT&T < Contes</title>{}",
        item(
            "a",
            "<description>Pierre & Paul &amp; Jacques &eacute;t&eacute; &#233; &inconnue;</description>"
        )
        .replace(
            "https://cdn.example/a.mp3",
            "https://cdn.example/a.mp3?x=1&y=2"
        )
    ));
    let feed = parse(&xml).unwrap();
    assert_eq!(feed.title, "AT&T < Contes");
    let ep = &feed.episodes[0];
    assert_eq!(ep.enclosure_url, "https://cdn.example/a.mp3?x=1&y=2");
    assert_eq!(
        ep.description.as_deref(),
        Some("Pierre & Paul & Jacques été é")
    );
}

#[test]
fn le_pretraitement_laisse_cdata_et_commentaires_intacts() {
    let text = "<a><![CDATA[x & y < z]]><!-- a & b --><?pi & ?>&amp; &#x41;</a>";
    assert!(matches!(sanitize_xml(text), Cow::Borrowed(_)));
    assert_eq!(
        sanitize_xml("<a>1 & 2 <br> <img src=\"u?a=1&b=2\"> <br/></a>"),
        "<a>1 &amp; 2 <br/> <img src=\"u?a=1&amp;b=2\"/> <br/></a>"
    );
    let feed = parse(&wrap(&item(
        "c",
        "<description><![CDATA[AT&T < 3]]></description>",
    )))
    .unwrap();
    assert_eq!(feed.episodes[0].description.as_deref(), Some("AT&T < 3"));
}

#[test]
fn le_pretraitement_suit_les_guillemets_des_attributs() {
    // `>` et `<` entre guillemets (doubles ou simples) ne ferment ni n'ouvrent la balise.
    assert_eq!(
        sanitize_xml("<a><img alt=\"a > b\" src=\"u\"> <br> </a>"),
        "<a><img alt=\"a > b\" src=\"u\"/> <br/> </a>"
    );
    assert_eq!(
        sanitize_xml("<a><img alt='a > b' src='u'>x</a>"),
        "<a><img alt='a > b' src='u'/>x</a>"
    );
    assert_eq!(
        sanitize_xml("<a><br title=\"x < y\"> z < t</a>"),
        "<a><br title=\"x < y\"/> z &lt; t</a>"
    );
    // Une apostrophe dans le texte n'ouvre aucune citation.
    assert_eq!(
        sanitize_xml("<a>l'été <br> d'or > 1</a>"),
        "<a>l'été <br/> d'or > 1</a>"
    );
    let feed = parse(&wrap(&item(
        "q",
        "<description>x<img alt=\"a > b\" title=\"c < d\">y</description>",
    )))
    .unwrap();
    assert_eq!(feed.episodes.len(), 1);
    assert_eq!(feed.episodes[0].description.as_deref(), Some("xy"));
}

#[test]
fn un_guid_trop_long_change_l_identifiant_d_episode() {
    let long = format!("urn:{}", "g".repeat(MAX_GUID_CHARS + 10));
    let feed = parse(&wrap(&item(&long, ""))).unwrap();
    let guid = &feed.episodes[0].guid;
    assert_eq!(guid.chars().count(), MAX_GUID_CHARS);
    assert_ne!(guid, &long);
    // Un identifiant calculé avant la limite (sur le `guid` brut) ne retrouve plus l'épisode :
    // la validation de pack le signale « épisode retiré du flux » (voir `pack::validate`).
    assert_ne!(
        crate::podcast::source::episode_id(guid),
        crate::podcast::source::episode_id(&long)
    );
}

#[test]
fn html_non_ferme_dans_un_xml_est_corrige_pour_les_balises_vides() {
    let feed = parse(&wrap(&item(
        "b",
        "<description>Ligne un<br>Ligne deux<hr>Fin</description>",
    )))
    .unwrap();
    assert_eq!(
        feed.episodes[0].description.as_deref(),
        Some("Ligne un\nLigne deux\nFin")
    );
}

#[test]
fn balise_html_jamais_fermee_garde_la_suite() {
    assert_eq!(strip_html("avant <b texte apres"), "avant <b texte apres");
    assert_eq!(strip_html("<p>un</p> a <i"), "un\na <i");
    // Entrée pathologique : pas de coût quadratique.
    let long = "<a".repeat(30_000);
    assert_eq!(strip_html(&long).len(), 60_000);
}

#[test]
fn fuseau_illisible_donne_une_date_inconnue() {
    for raw in [
        "Tue, 03 Jun 2008 11:05:30 +2",
        "Tue, 03 Jun 2008 11:05:30 GMT+2",
        "Tue, 03 Jun 2008 11:05:30 XYZ",
        "2008-06-03T11:05:30+2",
        "2008-06-03T11:05:30+99:99",
    ] {
        assert_eq!(parse_pub_date(raw), None, "{raw}");
    }
    let ok = [
        ("Tue, 03 Jun 2008 11:05:30 +0200", "2008-06-03T09:05:30Z"),
        ("Tue, 03 Jun 2008 11:05:30 GMT+0200", "2008-06-03T09:05:30Z"),
        ("Tue, 03 Jun 2008 11:05:30 UT", "2008-06-03T11:05:30Z"),
        ("Tue, 03 Jun 2008 11:05:30", "2008-06-03T11:05:30Z"),
        ("2008-06-03T11:05:30+02", "2008-06-03T09:05:30Z"),
        ("2008-06-03T11:05:30-05:00", "2008-06-03T16:05:30Z"),
        ("2008-06-03T11:05:30Z", "2008-06-03T11:05:30Z"),
    ];
    for (raw, iso) in ok {
        assert_eq!(parse_pub_date(raw).as_deref(), Some(iso), "{raw}");
    }
}

#[test]
fn annees_a_deux_chiffres_autour_de_49_et_50() {
    let date = |y: &str| parse_pub_date(&format!("03 Jun {y} 10:00:00 GMT"));
    assert_eq!(date("49").as_deref(), Some("2049-06-03T10:00:00Z"));
    assert_eq!(date("50").as_deref(), Some("1950-06-03T10:00:00Z"));
    assert_eq!(date("00").as_deref(), Some("2000-06-03T10:00:00Z"));
    assert_eq!(date("99").as_deref(), Some("1999-06-03T10:00:00Z"));
}

#[test]
fn le_29_fevrier_n_existe_que_les_annees_bissextiles() {
    assert!(parse_pub_date("29 Feb 2024 10:00:00 GMT").is_some());
    assert_eq!(parse_pub_date("29 Feb 2023 10:00:00 GMT"), None);
    assert_eq!(parse_pub_date("29 Feb 1900 10:00:00 GMT"), None);
    assert!(parse_pub_date("29 Feb 2000 10:00:00 GMT").is_some());
    assert_eq!(parse_pub_date("2023-02-29"), None);
}

#[test]
fn items_imbriques_ne_creent_pas_d_episode() {
    let xml = wrap(&format!(
        "<title>T</title><item><title>Externe</title><guid>x</guid>\
         <item><title>Interne</title><guid>y</guid>\
         <enclosure url=\"https://cdn.example/y.mp3\" type=\"audio/mpeg\"/></item>\
         <enclosure url=\"https://cdn.example/x.mp3\" type=\"audio/mpeg\"/></item>{}",
        item("z", "")
    ));
    let feed = parse(&xml).unwrap();
    let guids: Vec<&str> = feed.episodes.iter().map(|e| e.guid.as_str()).collect();
    assert_eq!(guids, ["x", "z"]);
    assert_eq!(feed.episodes[0].title, "Externe");
}

#[test]
fn prefixes_de_namespace_libres_et_attributs_insensibles_a_la_casse() {
    let xml = r#"<?xml version="1.0"?>
<rss version="2.0" xmlns:it="http://www.itunes.com/dtds/podcast-1.0.dtd"
     xmlns:d="http://purl.org/dc/elements/1.1/">
<channel><title>T</title><it:author>Chaîne</it:author><it:explicit>yes</it:explicit>
<item><title>Un</title><guid>1</guid><it:duration>01:00</it:duration>
  <d:creator>Dédé</d:creator><it:IMAGE HREF="https://img.example/1.png"/>
  <enclosure URL="https://cdn.example/1.mp3" TYPE="audio/mpeg" Length="9"/></item>
<item xmlns:itunes="http://autre.example/ns"><title>Deux</title><guid>2</guid>
  <itunes:duration>02:00</itunes:duration>
  <enclosure url="https://cdn.example/2.mp3" type="audio/mpeg"/></item>
</channel></rss>"#;
    let feed = parse(xml).unwrap();
    let (a, b) = (&feed.episodes[0], &feed.episodes[1]);
    assert_eq!(a.duration, Some(60));
    assert_eq!(a.author.as_deref(), Some("Dédé"));
    assert_eq!(a.image.as_deref(), Some("https://img.example/1.png"));
    assert_eq!(a.enclosure_url, "https://cdn.example/1.mp3");
    assert_eq!(a.enclosure_length, Some(9));
    assert_eq!(a.explicit, Some(true));
    // `itunes` lié à un autre espace de noms : n'est pas l'extension iTunes.
    assert_eq!(b.duration, None);
    assert_eq!(b.author.as_deref(), Some("Chaîne"));
    // Préfixe usuel sans déclaration : toléré.
    let bare = r#"<rss version="2.0"><channel><title>T</title>
<item><title>U</title><itunes:duration>03:00</itunes:duration>
<enclosure url="https://cdn.example/u.mp3" type="audio/mpeg"/></item></channel></rss>"#;
    assert_eq!(parse(bare).unwrap().episodes[0].duration, Some(180));
}

#[test]
fn guid_long_reste_distinct_et_titre_de_repli_suit_les_episodes_conserves() {
    let (a, b) = ("g".repeat(600), format!("{}h", "g".repeat(599)));
    let xml = wrap(&format!(
        "<item><guid>{a}</guid><enclosure url=\"https://c.example/a.mp3\" type=\"audio/mpeg\"/></item>\
         <item><title>Sans audio</title></item>\
         <item><guid>{b}</guid><enclosure url=\"https://c.example/b.mp3\" type=\"audio/mpeg\"/></item>"
    ));
    let feed = parse(&xml).unwrap();
    assert_eq!(feed.episodes.len(), 2, "{:?}", feed.warnings);
    let (ga, gb) = (&feed.episodes[0].guid, &feed.episodes[1].guid);
    assert_ne!(ga, gb);
    assert!(ga.chars().count() <= MAX_GUID_CHARS);
    assert_eq!(feed.episodes[0].title, "Épisode 1");
    assert_eq!(feed.episodes[1].title, "Épisode 2");
    // Déterministe.
    assert_eq!(parse(&xml).unwrap().episodes[0].guid, *ga);
}

#[test]
fn variantes_courantes_de_l_uri_itunes_et_zones_nommees() {
    for uri in [
        "http://www.itunes.com/DTDs/Podcast-1.0.dtd",
        "https://www.itunes.com/dtds/podcast-1.0.dtd/",
        " http://www.itunes.com/dtds/podcast-1.0.dtd ",
    ] {
        let xml = format!(
            r#"<rss version="2.0" xmlns:itunes="{uri}"><channel><title>T</title>
<item><title>U</title><itunes:duration>03:00</itunes:duration><itunes:explicit>yes</itunes:explicit>
<enclosure url="https://cdn.example/u.mp3" type="audio/mpeg"/></item></channel></rss>"#
        );
        let ep = &parse(&xml).unwrap().episodes[0];
        assert_eq!((ep.duration, ep.explicit), (Some(180), Some(true)), "{uri}");
    }
    assert_eq!(
        parse_pub_date("Tue, 03 Jun 2008 11:05:30 CEST").as_deref(),
        Some("2008-06-03T09:05:30Z")
    );
    // Balises vides déjà fermées ou à fermeture explicite : inchangées.
    let desc = "<description>a<br></br>b<br />c<img src=\"x\"/>d</description>";
    let feed = parse(&wrap(&item("v", desc))).unwrap();
    assert!(feed.episodes[0].description.is_some());
}
