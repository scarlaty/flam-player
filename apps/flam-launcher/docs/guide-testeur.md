# Guide testeur : flam-launcher et Flam en firmware 2.x

Merci de tester ! Cette pré-version envoie des histoires sur une Flam branchée en USB. Sur les
Flam en **firmware 1.x**, la fonction a été validée sur un appareil réel ; sur les Flam en
**firmware 2.x**, elle est **expérimentale et n'a jamais été essayée sur un appareil** : c'est
justement ce que vous allez tester. Elle est **non officielle** : lisez
[les risques](flam-transfert.md#risques) avant de commencer.

Ce guide vise surtout les Flam en 2.x. Si votre Flam est en 1.x, le launcher demandera en plus
une clé d'histoire officielle (« mode carrier », avec un avertissement à accepter) : sans cette
clé, abstenez-vous de tester l'envoi.

Vous n'avez pas besoin d'être développeur. Comptez environ une heure.

## 1. Installer la pré-version

1. Téléchargez l'installateur `Flam Launcher_<version>_x64-setup.exe` depuis la page des
   pré-versions (<https://github.com/scarlaty/flam-player/releases>), ainsi que la valeur **SHA-256** indiquée dans ses notes.
2. **Vérifiez l'empreinte SHA-256** (cela prouve que le fichier n'a pas été altéré). Ouvrez
   le dossier de téléchargement, faites un clic droit dans une zone vide, puis « Ouvrir dans le
   Terminal » (ou PowerShell), et tapez :

   ```powershell
   Get-FileHash -Algorithm SHA256 ".\Flam Launcher_<version>_x64-setup.exe"
   ```

   La valeur affichée doit être **identique** à celle des notes de la pré-version. Sinon, ne
   lancez pas l'installateur.

3. Lancez l'installateur. Il n'est **pas signé** : Windows SmartScreen affiche « Windows a
   protégé votre ordinateur ». Cliquez sur **Informations complémentaires**, puis sur
   **Exécuter quand même**.
4. L'installation se fait pour votre compte Windows, sans droits d'administrateur. Lancez
   ensuite « Flam Launcher » depuis le menu Démarrer.

## 2. AVANT TOUT : copiez le contenu de votre Flam sur le PC

<a id="sauvegarde"></a>

Le launcher ne fait **pas** de sauvegarde complète de la Flam. Il ne garde que son fichier
`.mdf`. Sans copie de votre part, une erreur pourrait vous faire perdre vos histoires. **Ne
lancez aucun test avant d'avoir fait cette copie.**

1. Branchez la Flam au PC et activez le **mode connexion USB** dans ses réglages. Elle apparaît
   dans l'Explorateur de fichiers comme un disque (par exemple `E:`).
2. Dans l'Explorateur, affichez les éléments cachés : Windows 11 : menu **Afficher**, puis
   **Afficher**, puis cochez **Éléments masqués** (Windows 10 : onglet **Affichage**, cochez
   **Éléments masqués**). (Certains fichiers de la Flam, comme `.mdf`, peuvent être
   invisibles autrement.)
3. Créez un dossier sur votre PC, par exemple `C:\Sauvegarde-Flam-2026-10-20`.
4. Ouvrez le disque de la Flam, appuyez sur **Ctrl + A** (tout sélectionner), puis **Ctrl + C**
   (copier). Ouvrez votre dossier de sauvegarde et appuyez sur **Ctrl + V** (coller).
   La copie peut durer plusieurs minutes : attendez la fin.
5. **Vérifiez la copie** : ouvrez le disque de la Flam, **Ctrl + A**, clic droit sur la
   sélection, **Propriétés**, et notez « Contient : N fichiers, M dossiers » et la **Taille**
   (pas « Taille sur disque »). Faites exactement la même chose dans votre dossier de
   sauvegarde (éléments masqués affichés des deux côtés). Les nombres doivent être identiques
   et les tailles égales. Si Windows a signalé une erreur pendant la copie, ou si les nombres
   diffèrent, recommencez. Vérifiez que la copie contient les dossiers `str` et `etc` et le
   fichier `.mdf`.
6. Gardez ce dossier jusqu'à la fin de la bêta. Ne le modifiez pas.

### Restaurer

<a id="restaurer"></a>

Si la Flam ne fonctionne plus correctement après un test :

Restauration ciblée, sur ce que le launcher modifie (à privilégier) :

1. Fermez le launcher. Branchez la Flam, mode connexion USB activé.
2. Sur la Flam, supprimez : le dossier `str`, le dossier `str.hidden` (s'il existe), le dossier
   `etc\library`, les dossiers `.flam-transfer` et `.flam-trash` (s'ils existent) et le fichier
   `usr\0\library.cache` (s'il existe).
3. Depuis votre sauvegarde, recopiez sur la Flam, au même emplacement, `str`, `str.hidden`
   (s'il existe) et `etc\library` seulement.
4. Ne recopiez **jamais** le fichier `.mdf` ni les autres dossiers de `etc` (réglages de
   l'appareil, liés à la version du firmware), sauf en dernier recours.
5. Éjectez la Flam, débranchez-la, puis redémarrez-la.

Dernier recours (la Flam ne démarre plus ou ne se laisse plus lire) : recopiez tout le contenu
de la sauvegarde sur la Flam en remplaçant les fichiers existants. Cela n'est sûr que si le
firmware de la Flam n'a pas changé depuis la sauvegarde.

Ne **formatez** jamais la Flam.

## 3. Les tests

Notez pour chaque test ce que vous voyez, surtout si ce n'est pas ce qui est attendu. Une
seule histoire courte suffit pour les tests 3 à 6.

1. **Détection.** Branchez la Flam (mode connexion USB), ouvrez le launcher, onglet **Ma
   Flam**. Attendu : le bandeau « Fonction expérimentale et non officielle » et, pour une Flam
   en 2.x, « Firmware 2.x : en test, sauvegardez votre Flam avant. » ; la carte « Flam
   détectée » avec le disque, le **firmware** (notez-le ; « illisible » est à signaler) et
   l'espace libre. Débranchez puis rebranchez la Flam : elle doit réapparaître d'elle-même
   (sinon, cliquez sur **Réessayer**).
2. **Ma Flam : titres et vignettes.** Attendu : la liste de vos histoires avec leur titre et
   leur image. Une histoire sans titre est affichée « Histoire » suivie d'un identifiant : notez
   combien et lesquelles. Notez aussi tout avertissement « À noter » ou sur l'index.
3. **Envoyer une histoire.** Dans « Mes histoires » (installez d'abord une histoire depuis une
   source si besoin), cliquez sur **Envoyer sur la Flam**, lisez l'avertissement s'il s'affiche et confirmez.
   Attendu : une progression (copie, puis vérification) jusqu'à « Envoi terminé ». **Ne
   débranchez pas la Flam pendant l'envoi.** Notez la durée et la taille de l'histoire.
4. **La lancer sur la Flam.** Éjectez la Flam (test 7), débranchez-la, **redémarrez-la**, puis
   cherchez l'histoire dans la liste de la Flam et lancez-la. Attendu : titre lisible, images,
   sons, histoire jouable jusqu'au bout. Notez tout écran noir, titre illisible ou blocage.
5. **Retirer.** Rebranchez la Flam (mode connexion USB activé), onglet **Ma Flam**, cliquez sur **Retirer** à côté de
   l'histoire envoyée, confirmez. Attendu : elle disparaît de la liste ; après redémarrage de la
   Flam, elle a disparu aussi de la Flam. Vérifiez que vos autres histoires sont intactes.
6. **Réordonner.** Si vous avez au moins deux histoires, cliquez sur **Modifier l'ordre**,
   déplacez une histoire (glisser-déposer ou boutons), puis **Enregistrer l'ordre**. Attendu :
   après redémarrage, la Flam affiche le nouvel ordre.
7. **Éjecter.** Cliquez sur **Éjecter la Flam**. Attendu : « Vous pouvez débrancher la Flam. »
   Si l'éjection est refusée, notez le message (fermez les fenêtres de l'Explorateur ouvertes sur
   la Flam et réessayez).
8. **Redémarrer.** Débranchez, redémarrez la Flam (éteignez-la puis rallumez-la). Vérifiez que
   toutes vos histoires d'origine sont toujours là et fonctionnent.

Facultatif, si vous le souhaitez : une seconde histoire plus volumineuse, ou plusieurs envois
d'affilée, et notez la durée.

### Points à vérifier en 2.x

<!-- À compléter avec les résultats de l'audit #317 (chemin firmware 2.x). -->

_À venir : points précis à observer sur une Flam en firmware 2.x. En attendant, notez tout ce
qui vous semble anormal._

## 4. Remonter votre retour

Quel que soit le résultat (réussite comprise : un « tout est OK » est précieux), ouvrez une
issue sur GitHub :

1. Allez sur <https://github.com/scarlaty/flam-player/issues/new/choose> (un compte GitHub
   est nécessaire) et choisissez le modèle **« Retour testeur Flam »** (si ce modèle n'existe pas encore dans
   votre version, voir plus bas).
2. Remplissez le modèle : modèle de Flam, version du firmware, numéro de test, ce que vous avez
   vu.
3. Dans « Ma Flam », cliquez sur **Copier le diagnostic** (si ce bouton n'existe pas encore)
   et collez le résultat dans l'issue. Il aide à comprendre ce qui s'est passé.
4. Ne publiez **jamais** de clé, de fichier `.mdf` ni de contenu d'histoire officielle.

Si le modèle ou le bouton ne sont pas encore disponibles dans votre version, décrivez votre
retour avec vos mots : la version du launcher (écran « Paramètres », rubrique « À propos »), le firmware de la
Flam (affiché sur la carte « Flam détectée ») et les messages affichés à l'écran.

## En cas de problème

Voir le [dépannage](flam-transfert.md#dépannage). En cas de doute, **ne continuez pas** :
restaurez votre copie (section « Restaurer ») puis signalez le problème.
