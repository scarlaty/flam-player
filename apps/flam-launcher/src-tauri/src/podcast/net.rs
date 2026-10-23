//! Contrôle des adresses réseau des téléchargements de podcasts (D10, D11).
//!
//! Le flux, les enclosures et les images sont des adresses saisies ou publiées par des tiers :
//! sans garde, elles permettraient de faire sonder le réseau local (routeur, NAS, service de
//! métadonnées d'un hébergeur) par l'application. Ce module refuse les adresses privées et
//! locales **partout** :
//!
//! - adresse littérale dans l'URL initiale et dans **chaque redirection** ([`check_url`],
//!   [`guarded_client`]) ;
//! - nom de domaine : toutes les adresses obtenues par la résolution DNS sont vérifiées, et la
//!   connexion se fait **sur ces adresses vérifiées** (résolveur du client, aucune seconde
//!   résolution possible : pas de « DNS rebinding ») ;
//! - équivalents IPv6 : IPv4 encapsulée (`::ffff:a.b.c.d`, NAT64, 6to4), lien local, ULA…
//!
//! Le client refuse aussi les mandataires (`HTTP_PROXY`…) : un mandataire résoudrait lui-même
//! le nom, hors de ce contrôle.
//!
//! # Brancher le téléchargement du flux (#211)
//!
//! ```ignore
//! let policy = NetPolicy::strict();
//! check_url(&url, policy)?;                       // adresse littérale
//! let client = guarded_client(policy, connect, read_timeout)?;
//! let response = client.get(url).send().await;    // DNS et redirections contrôlés
//! if let Err(e) = &response { if is_blocked(e) { /* adresse refusée */ } }
//! ```
//!
//! # URL secrètes (D11)
//!
//! Une URL de flux ou d'enclosure peut contenir un jeton : elle n'est jamais journalisée en
//! entier. [`redact_url`] donne la seule forme affichable (schéma et hôte).

use std::error::Error;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use url::{Host, Url};

use crate::catalog::{redirect_policy, USER_AGENT};

/// Politique réseau. Par défaut (et en production) : [`NetPolicy::strict`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NetPolicy {
    /// Autorise le bouclage (127/8, `::1`). Champ privé, constructeur réservé aux tests : les
    /// serveurs de test écoutent sur `127.0.0.1`. Les autres plages restent refusées.
    allow_loopback: bool,
}

impl NetPolicy {
    /// Toutes les adresses privées et locales sont refusées.
    pub const fn strict() -> Self {
        Self {
            allow_loopback: false,
        }
    }

    /// Tests seulement : autorise le bouclage pour joindre un serveur local de test.
    #[cfg(test)]
    pub const fn loopback_for_tests() -> Self {
        Self {
            allow_loopback: true,
        }
    }

    /// `true` si une connexion à `ip` est permise.
    pub fn allows(self, ip: IpAddr) -> bool {
        if !is_forbidden_ip(ip) {
            return true;
        }
        self.allow_loopback && ip.to_canonical().is_loopback()
    }
}

/// Adresse refusée par D10. Ne contient aucune URL ni adresse (D11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockedAddress;

impl fmt::Display for BlockedAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("adresse refusée : réseau privé, local ou non public")
    }
}

impl Error for BlockedAddress {}

/// `true` si `err` (ou l'une de ses causes) est un refus d'adresse.
pub fn is_blocked(err: &(dyn Error + 'static)) -> bool {
    let mut current: Option<&(dyn Error + 'static)> = Some(err);
    while let Some(e) = current {
        if e.is::<BlockedAddress>() {
            return true;
        }
        current = e.source();
    }
    false
}

fn forbidden_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || a == 0
        // Partagée (CGNAT) 100.64.0.0/10.
        || (a == 100 && (64..=127).contains(&b))
        // Réservée à l'IETF 192.0.0.0/24, documentation 192.0.2/24, 198.51.100/24, 203.0.113/24.
        || (a == 192 && b == 0 && (c == 0 || c == 2))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        // Tests de performance 198.18.0.0/15.
        || (a == 198 && (b == 18 || b == 19))
        // Relais 6to4 192.88.99.0/24 (obsolète).
        || (a == 192 && b == 88 && c == 99)
        // Classe E réservée 240.0.0.0/4.
        || a >= 240
}

fn forbidden_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return forbidden_v4(v4);
    }
    let s = ip.segments();
    let embedded = |hi: u16, lo: u16| {
        let [a, b] = hi.to_be_bytes();
        let [c, d] = lo.to_be_bytes();
        Ipv4Addr::new(a, b, c, d)
    };
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        // Lien local fe80::/10, site local fec0::/10.
        || (s[0] & 0xffc0) == 0xfe80
        || (s[0] & 0xffc0) == 0xfec0
        // Locale unique fc00::/7.
        || (s[0] & 0xfe00) == 0xfc00
        // Documentation 2001:db8::/32, Teredo 2001::/32 (tunnel), discard 100::/64.
        || (s[0] == 0x2001 && (s[1] == 0x0db8 || s[1] == 0))
        || (s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 0)
        // IPv4 compatible ::a.b.c.d (obsolète) : on juge l'IPv4 contenue.
        || (s[..6].iter().all(|x| *x == 0) && forbidden_v4(embedded(s[6], s[7])))
        // NAT64 64:ff9b::/96 : l'IPv4 contenue décide.
        || (s[0] == 0x0064 && s[1] == 0xff9b && s[2..6].iter().all(|x| *x == 0)
            && forbidden_v4(embedded(s[6], s[7])))
        // NAT64 local 64:ff9b:1::/48 (RFC 8215) : refusé.
        || (s[0] == 0x0064 && s[1] == 0xff9b && s[2] == 1)
        // SIIT ::ffff:0:a.b.c.d : l'IPv4 contenue décide.
        || (s[..4].iter().all(|x| *x == 0) && s[4] == 0xffff && s[5] == 0
            && forbidden_v4(embedded(s[6], s[7])))
        // 6to4 2002::/16 : l'IPv4 contenue décide.
        || (s[0] == 0x2002 && forbidden_v4(embedded(s[1], s[2])))
}

/// `true` si `ip` n'est pas une adresse publique (D10) : bouclage, privée, lien local,
/// partagée 100.64/10, multidiffusion, non spécifiée, réservée, et équivalents IPv6.
pub fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => forbidden_v4(v4),
        IpAddr::V6(v6) => forbidden_v6(v6),
    }
}

/// Contrôle une URL **avant** toute connexion : schéma `http`/`https`, hôte présent, et
/// adresse IP littérale publique. Un nom de domaine est contrôlé à la résolution DNS
/// ([`guarded_client`]).
pub fn check_url(url: &Url, policy: NetPolicy) -> Result<(), BlockedAddress> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(BlockedAddress);
    }
    match url.host() {
        None => Err(BlockedAddress),
        Some(Host::Domain(_)) => Ok(()),
        Some(Host::Ipv4(ip)) if policy.allows(IpAddr::V4(ip)) => Ok(()),
        Some(Host::Ipv6(ip)) if policy.allows(IpAddr::V6(ip)) => Ok(()),
        Some(_) => Err(BlockedAddress),
    }
}

/// Garde des adresses résolues : refuse **tout** le résultat si une seule adresse est
/// interdite (une réponse mixte est suspecte), ou s'il est vide.
pub fn filter_resolved(
    addrs: Vec<SocketAddr>,
    policy: NetPolicy,
) -> Result<Vec<SocketAddr>, BlockedAddress> {
    if addrs.is_empty() || addrs.iter().any(|a| !policy.allows(a.ip())) {
        return Err(BlockedAddress);
    }
    Ok(addrs)
}

/// Résout `host` puis vérifie les adresses obtenues.
pub async fn resolve_host(
    host: &str,
    policy: NetPolicy,
) -> Result<Vec<SocketAddr>, Box<dyn Error + Send + Sync>> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, 0)).await?.collect();
    Ok(filter_resolved(addrs, policy)?)
}

struct GuardedResolver {
    policy: NetPolicy,
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let policy = self.policy;
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs = resolve_host(&host, policy).await?;
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

fn guarded_redirect(
    attempt: reqwest::redirect::Attempt<'_>,
    policy: NetPolicy,
) -> reqwest::redirect::Action {
    if check_url(attempt.url(), policy).is_err() {
        return attempt.error(BlockedAddress);
    }
    redirect_policy(attempt)
}

/// Client HTTP des téléchargements de podcasts : rustls (TLS vérifié), 5 redirections au
/// plus (jamais https vers http), adresses contrôlées à chaque redirection et après
/// résolution DNS, connexion à l'adresse vérifiée, aucun mandataire. Pas de délai total
/// (fichiers de plusieurs dizaines de Mio) : `read_timeout` borne les silences.
pub fn guarded_client(
    policy: NetPolicy,
    connect_timeout: Duration,
    read_timeout: Duration,
) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .use_rustls_tls()
        .connect_timeout(connect_timeout)
        .read_timeout(read_timeout)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            guarded_redirect(attempt, policy)
        }))
        .dns_resolver(Arc::new(GuardedResolver { policy }))
        .no_proxy()
        .user_agent(USER_AGENT)
        .build()
}

/// Forme journalisable d'une URL (D11) : `schéma://hôte/…`, sans chemin, requête, jeton ni
/// identifiants. Une URL illisible donne un texte neutre.
pub fn redact_url(url: &str) -> String {
    match Url::parse(url) {
        Ok(u) => match u.host_str() {
            Some(host) => format!("{}://{}/…", u.scheme(), host),
            None => "<url sans hôte>".to_string(),
        },
        Err(_) => "<url illisible>".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn adresses_refusees() {
        for s in [
            "127.0.0.1",
            "127.255.0.3",
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.255",
            "224.0.0.1",
            "239.255.255.250",
            "255.255.255.255",
            "240.0.0.1",
            "192.0.0.8",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "::",
            "::1",
            "fe80::1",
            "fc00::1",
            "fd12:3456::1",
            "fec0::1",
            "ff02::1",
            "2001:db8::1",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
            "::ffff:127.0.0.1",
            "::ffff:10.1.2.3",
            "::ffff:169.254.169.254",
            "::10.0.0.1",
            "64:ff9b::a00:1",
            "64:ff9b::7f00:1",
            "2002:a00:1::1",
            "2002:7f00:1::",
            "64:ff9b:1::a00:1",
            "::ffff:0:a00:1",
            "192.88.99.1",
        ] {
            assert!(is_forbidden_ip(ip(s)), "{s} devrait être refusée");
        }
    }

    #[test]
    fn adresses_publiques_acceptees() {
        for s in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "100.63.255.255",
            "100.128.0.1",
            "172.15.255.255",
            "172.32.0.1",
            "192.169.0.1",
            "198.17.255.255",
            "198.20.0.1",
            "223.255.255.255",
            "2606:4700:4700::1111",
            "2a00:1450:4007:80f::200e",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
            "2002:808:808::1",
        ] {
            assert!(!is_forbidden_ip(ip(s)), "{s} devrait être acceptée");
        }
    }

    #[test]
    fn politique_de_test_n_autorise_que_le_bouclage() {
        let test = NetPolicy::loopback_for_tests();
        assert!(test.allows(ip("127.0.0.1")));
        assert!(test.allows(ip("::1")));
        assert!(test.allows(ip("::ffff:127.0.0.1")));
        assert!(!test.allows(ip("10.0.0.1")));
        assert!(!test.allows(ip("169.254.169.254")));
        assert!(!test.allows(ip("fd00::1")));
        let strict = NetPolicy::strict();
        assert!(!strict.allows(ip("127.0.0.1")));
        assert_eq!(NetPolicy::default(), strict);
    }

    #[test]
    fn url_litterale_controlee() {
        let strict = NetPolicy::strict();
        let check = |u: &str| check_url(&Url::parse(u).unwrap(), strict);
        assert!(check("https://example.org/a.mp3").is_ok());
        assert!(check("http://8.8.8.8/a.mp3").is_ok());
        assert!(check("http://127.0.0.1/a").is_err());
        assert!(
            check("http://localhost.:80/a").is_ok(),
            "nom : contrôlé au DNS"
        );
        assert!(check("http://[::1]/a").is_err());
        assert!(check("http://[::ffff:192.168.0.1]/a").is_err());
        assert!(check("http://169.254.169.254/latest").is_err());
        // Formes décimale, octale, hexadécimale : normalisées par `url` en IPv4.
        assert!(check("http://2130706433/").is_err());
        assert!(check("http://0x7f.1/").is_err());
        assert!(check("http://017700000001/").is_err());
        assert!(check("ftp://example.org/a").is_err());
        assert!(check("file:///etc/passwd").is_err());
    }

    #[test]
    fn reponse_dns_mixte_ou_vide_refusee() {
        let strict = NetPolicy::strict();
        let sa = |s: &str| SocketAddr::new(ip(s), 0);
        assert!(filter_resolved(vec![sa("8.8.8.8"), sa("1.1.1.1")], strict).is_ok());
        assert!(filter_resolved(vec![sa("8.8.8.8"), sa("10.0.0.5")], strict).is_err());
        assert!(filter_resolved(vec![sa("fd00::1")], strict).is_err());
        assert!(filter_resolved(vec![], strict).is_err());
    }

    #[tokio::test]
    async fn resolution_de_localhost_refusee_sauf_test() {
        assert!(resolve_host("localhost", NetPolicy::strict())
            .await
            .is_err());
        let err = resolve_host("localhost", NetPolicy::strict())
            .await
            .unwrap_err();
        assert!(is_blocked(err.as_ref()));
        let ok = resolve_host("localhost", NetPolicy::loopback_for_tests())
            .await
            .unwrap();
        assert!(!ok.is_empty());
    }

    #[test]
    fn url_masquee() {
        assert_eq!(
            redact_url("https://user:pw@feeds.example.org:8443/p/abc123token?key=SECRET#x"),
            "https://feeds.example.org/…"
        );
        assert_eq!(redact_url("pas une url"), "<url illisible>");
        assert!(!redact_url("https://h.example/SECRET?t=SECRET").contains("SECRET"));
    }
}
