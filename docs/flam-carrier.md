# Mode « carrier » : Flam en firmware 1.x

Issues : #234, #271 (saisie du `bt`), épopée : #222. Code : `apps/flam-launcher/src-tauri/src/device/carrier.rs`
(clés, cohérence, conservation), `device/transfer.rs` (envoi), `src/features/device/`
(interface). Ce document ne contient **aucune clé**.

## 1. Avertissement (version 1)

Texte validé par le mainteneur, affiché tel quel avant la première utilisation, avec une case
« J'ai compris » obligatoire. L'acceptation est enregistrée dans les réglages
(`carrier_consent` : version du texte et date UTC). Tout changement du texte incrémente
`WARNING_VERSION` dans `carrier.rs` et redemande le consentement ; un test vérifie que le texte
ci-dessous est identique à celui du code.

> Fonction expérimentale, non officielle. Pour installer une histoire sur une Flam en firmware 1.x, flam-launcher réutilise les clés d'une histoire officielle déjà présente sur cette Flam et achetée sur votre compte. Aucune clé n'est fournie par flam-launcher, aucune histoire officielle n'est copiée ni exportée. Si cette histoire officielle est retirée de la Flam ou de votre compte, les histoires installées ainsi peuvent devenir illisibles. Usage strictement personnel, à vos risques ; fonction ni approuvée ni prise en charge par Lunii.

## 2. Principe (supposé, à confirmer sur appareil : #236)

Le firmware 1.x ne lit pas une histoire chiffrée avec le matériel de `.mdf`. Il lit une
histoire chiffrée avec le couple d'une histoire officielle **du même compte** :

| Fichier de l'histoire officielle | Taille utile | Usage |
|---|---|---|
| `key` | 32 octets | copié tel quel dans `str/<uuid>/key` de l'histoire installée |
| `bt` | 32 octets : clé AES-128 (16) puis IV (16) | chiffre `info` et les scripts `*.lsf` (AES-128-CBC, bourrage nul) |

Images, sons et `version` restent en clair, comme en 2.x. `bt` n'est jamais copié sur la Flam
ni ailleurs (hors conservation chiffrée, § 4).

## 3. Source du couple

Au choix de l'utilisateur, **après** consentement :

1. une histoire officielle de la Flam qui porte `bt` et `key` et dont `bt` déchiffre son propre
   `info` en texte lisible (liste avec titres). Avec un couple conservé pour cette Flam (§ 4),
   la liste montre aussi les histoires **sans** `bt` dont `info` se déchiffre avec le `bt`
   conservé : le couple est alors ce `bt` et le `key` de l'histoire choisie ;
2. un fichier : le `bt` d'une copie de dossier d'histoire (avec `key` dans le même dossier), ou
   une archive zip contenant exactement un `bt` et le `key` du même dossier ;
3. le couple conservé pour cette Flam (§ 4) ;
4. la **saisie** du `bt` (#271) : 64 caractères hexadécimaux (espaces et majuscules tolérés),
   dans un champ masqué (case « Afficher la clé »). Le front n'active l'envoi qu'avec une saisie
   valide ; le backend la revalide (`EnteredBt` : `Debug` masqué, jamais reprise dans un
   message d'erreur ; la copie qu'il garde est effacée de la mémoire à la destruction, mais pas
   les copies intermédiaires faites par le transport IPC et la désérialisation, hors de son
   contrôle). Il parcourt ensuite les histoires de la Flam (`str/` puis `str.hidden/`, au plus
   5000, annulation vérifiée à chaque histoire) et retient la première dont `info` se déchiffre
   avec ce `bt` ; son `key` complète le couple. Aucune : refus « cette clé ne correspond à
   aucune histoire officielle de cette Flam », rien n'est écrit ni sur la Flam ni sur le PC.
   Une clé saisie sans « Conserver ces clés » n'est pas gardée par l'interface : elle est
   redemandée à l'envoi suivant.

   **L'IV n'est pas vérifiable de façon certaine.** En AES-CBC, l'IV (seconde moitié du `bt`)
   n'agit que sur le premier bloc de 16 octets du clair : une faute de frappe dans l'IV altère
   quelques octets du début du titre et laisse le reste intact, si bien qu'un contrôle de
   lisibilité peut l'accepter, de même que la relecture après écriture (même couple faux).
   Envoyée avec un tel `bt`, l'histoire aurait le début de chaque fichier chiffré faux. D'où :
   - un contrôle strict du premier bloc, pour toutes les sources : UTF-8 strict, sans caractère
     de remplacement ni caractère de contrôle autre que `\n` (ou le `\r` d'un `\r\n`). Il
     écarte une partie des IV faux, pas tous ;
   - une **confirmation du titre** : « Vérifier la clé » (`device_carrier_preview`, lecture
     seule, consentement exigé, verrou de l'appareil tenu, annulable) renvoie le seul titre de
     l'histoire reconnue, jamais de clé. Le dialogue affiche « Clé reconnue : histoire « … » »
     et n'active l'envoi (et la conservation) qu'après la case « Le titre est exact ». Le
     backend refuse une saisie non confirmée (`confirmed`). Un IV faux altère le début du
     titre : c'est visible ;
   - **copier-coller la clé** plutôt que la recopier à la main est recommandé.

   La structure supposée de `info` (titre répété en quatrième ligne, hors du premier bloc)
   permettrait une vérification exacte de l'IV, mais elle n'est connue que par un jeu de test :
   elle n'est pas utilisée tant qu'elle n'est pas confirmée sur appareil (C7, § 6).

Sur la Flam 1.x du mainteneur, **aucun** dossier d'histoire ne contient de `bt` (seulement
`info`, `key`, `main.lsf`, `version`) : la liste est vide à la première utilisation et le
dialogue l'indique (« Aucune histoire de cette Flam ne contient de fichier bt : saisissez la clé
ou choisissez un fichier. ») ; la saisie est alors proposée d'office.

**Cohérence, avant toute copie** : une histoire de la Flam (`str/` ou `str.hidden/`) doit avoir
le même `key` et un `info` que le couple déchiffre en texte lisible (UTF-8 strict, au moins
90 % de caractères imprimables). Sinon : refus « couple invalide », rien n'est écrit (ni sur la
Flam ni dans la préparation). **Après écriture**, `info` de l'histoire installée est relu sur la
Flam et doit se déchiffrer avec le couple (`carrier_readback` du rapport).

## 4. Conservation sur le PC

- Fichier `<données locales>/flam-launcher/carrier-keys/<empreinte>.dpapi`, un par Flam.
  L'empreinte est un SHA-256 tronqué du champ du numéro de série de `.mdf` : ni le numéro ni
  une clé n'apparaissent dans le nom.
- Contenu chiffré par DPAPI (`CryptProtectData`, portée utilisateur, sans invite), avec une
  entropie liée à l'application et à la Flam : un fichier copié d'une Flam à l'autre, ou ouvert
  par un autre compte Windows, est refusé.
- Écriture atomique (fichier temporaire puis renommage). « Oublier les clés » supprime le
  fichier de la Flam branchée.
- Conservation facultative (case « Conserver ces clés sur ce PC ») ; sans elle, le choix est
  redemandé.

Les clés ne figurent jamais dans le dépôt, les journaux, les messages d'erreur, les réponses
IPC ni les exports : `Debug` les masque, la mémoire est effacée (`zeroize`), et des tests
recherchent leurs octets dans toutes ces sorties.

## 5. Invariants et coupures

- Aucune lecture de `bt` ni copie sans consentement à la version courante du texte (refus
  côté backend, quel que soit l'appel du front).
- Aucune écriture sur la Flam avant la vérification de cohérence ; le transfert reprend ensuite
  le déroulé sûr de #229 (dossier temporaire, vérification, renommage, index).

| Arrêt ou échec pendant… | État laissé | Reprise |
|---|---|---|
| le consentement, le choix ou la vérification | rien d'écrit | aucune |
| la conservation chiffrée | ancien fichier intact ou nouveau complet (renommage atomique) ; échec signalé au journal, l'envoi continue | aucune |
| la préparation (PC) | chantier `.partiel-*` | `cleanup_stale` au prochain envoi |
| la copie, la vérification, le renommage, l'index | voir le tableau de `transfer.rs` (#229) | `recover` au branchement |
| la relecture de `info` | histoire installée ; `carrier_readback` faux signalé | retirer l'histoire (#231) |

## 6. À valider sur la Flam du mainteneur (#236)

- C1 : `bt` et `key` présents dans les histoires officielles en 1.x ; tailles réelles.
  Observé : `key` de 32 octets, `info` de 32 à 144 octets, **pas de `bt`** (d'où la saisie,
  #271).
- C2 : une histoire chiffrée avec `bt` et munie du `key` officiel est lue (titre lisible,
  lancement) après redémarrage.
- C3 : position du numéro de série dans `.mdf` en 1.x (identifiant de conservation stable).
- C4 : histoires masquées (`str.hidden/`) utilisables comme source.
- C5 : comportement si l'histoire officielle est retirée de la Flam ou du compte.
- C6 : un couple d'un autre compte est bien refusé par le firmware (et par la vérification).
- C7 : structure du clair de `info` (titre, sous-titre, `000000`, titre ?) : si le titre est
  bien répété hors du premier bloc, il peut servir à vérifier exactement l'IV d'un `bt` saisi.
