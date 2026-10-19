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
   pré-versions (<https://github.com/scarlaty/flam-player/releases>), ainsi que le fichier `SHA256SUMS.txt` publié avec la pré-version (il contient l'empreinte
   **SHA-256** de l'installateur).
2. **Vérifiez l'empreinte SHA-256** (cela prouve que le fichier n'a pas été altéré). Ouvrez
   le dossier de téléchargement, faites un clic droit dans une zone vide, puis « Ouvrir dans le
   Terminal » (ou PowerShell), et tapez :

   ```powershell
   Get-FileHash -Algorithm SHA256 ".\Flam Launcher_<version>_x64-setup.exe"
   ```

   La valeur affichée doit être **identique** à celle de `SHA256SUMS.txt`. Sinon, ne
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

**Pendant toute la bêta, ne synchronisez pas la Flam avec l'application officielle et ne mettez
pas à jour son firmware.** Si cela arrive, refaites une copie complète avant de continuer.

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

**Si le firmware affiché dans « Ma Flam » a changé depuis votre copie, ne restaurez pas :
signalez-le.**

À chaque restauration, ne débranchez pas la Flam pendant la copie ; si Windows signale une
erreur, recommencez l'étape de copie ; vérifiez ensuite les nombres de fichiers comme à la
sauvegarde. Attention : ce que vous supprimez sur la Flam ne passe pas par une corbeille
(disque amovible) : assurez-vous que votre copie est complète avant de supprimer.

Restauration ciblée, sur ce que le launcher modifie (à privilégier) :

1. Fermez le launcher. Branchez la Flam, mode connexion USB activé.
2. Sur la Flam, supprimez : le dossier `str`, le dossier `str.hidden` (s'il existe), le dossier
   `etc\library`, les dossiers `.flam-transfer` et `.flam-trash` (s'ils existent) et le fichier
   `usr\0\library.cache` (s'il existe).
3. Depuis votre sauvegarde, recopiez sur la Flam, au même emplacement, `str`, `str.hidden`
   (s'il existe) et `etc\library` seulement.
4. Ne recopiez **jamais** le fichier `.mdf` ni les autres dossiers de `etc` (réglages de
   l'appareil, liés à la version du firmware), sauf en dernier recours (voir « Dernier recours » ci-dessous).
5. Éjectez la Flam depuis l'Explorateur, débranchez-la, puis redémarrez-la.

Dernier recours (la Flam ne démarre plus ou ne se laisse plus lire) : recopiez tout le contenu
de la sauvegarde sur la Flam. Supprimez d'abord sur la Flam `str`, `str.hidden`, `etc\library`,
`.flam-transfer` et `.flam-trash`, puis recopiez tout en remplaçant les fichiers existants. Cela
n'est sûr que si le firmware de la Flam n'a pas changé depuis la sauvegarde.

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

Ces points servent à confirmer, sur une vraie Flam 2.x, ce que le launcher suppose. Notez
chaque réponse dans votre retour, même si tout va bien.

1. **Ce qu'affiche « Ma Flam ».** Sur la carte « Flam détectée », la ligne **Firmware** donne la
   version, puis un état. Recopiez la ligne entière, par exemple
   « 2.4.2 (2.x) · en test, confirmation à donner ». Indiquez si la mention « fichier .mdf non
   standard » apparaît. La version doit être la même que dans les réglages de la Flam.
2. **La confirmation.** Avant le premier envoi, le launcher demande de cocher « Firmware 2.x en
   test : j'ai sauvegardé ma Flam ». Ne cochez qu'**après** avoir fait la sauvegarde (section 2).
   Notez si le texte vous a paru clair. Si un bandeau rouge « Écriture bloquée » ou « Envoi
   bloqué » s'affiche, recopiez son texte : rien n'est alors écrit sur la Flam.
3. **Les titres de vos histoires officielles.** Dans la liste de « Ma Flam », vos histoires
   achetées s'affichent-elles avec leur vrai titre, sous un identifiant, ou avec un titre venu
   d'internet ? Indiquez si c'est le cas pour toutes, quelques-unes ou aucune.
4. **La première histoire envoyée.** Envoyez **une seule** petite histoire. Éjectez la Flam,
   débranchez-la, **redémarrez-la** : l'histoire apparaît-elle à la fin de la liste, avec son
   titre lisible ? Se lance-t-elle, avec les images et le son ? Si elle apparaît mais refuse de
   se lancer, notez exactement ce qui se passe (message, écran figé, histoire absente).
5. **La vérification par la Flam.** Après ce redémarrage, rebranchez la Flam en mode connexion
   USB et ouvrez « Ma Flam ». La ligne Firmware indique-t-elle « vérifié par la Flam » ? Si elle
   indique toujours « sauvegarde confirmée », notez-le : ce n'est pas grave, mais c'est une
   information utile.
6. **Les dossiers de travail du launcher.** Si un envoi ou un retrait a été interrompu
   (débranchement), regardez à la racine de la Flam (fichiers cachés affichés) s'il reste un
   dossier `.flam-transfer` ou `.flam-trash`, ou dans `etc\library` un fichier `list.new` ou
   `list.bak`. La Flam démarre-t-elle quand même normalement ? Ces éléments doivent disparaître
   au branchement suivant.
7. **L'éjection.** Le bouton **Éjecter la Flam** fonctionne-t-il, ou faut-il passer par
   l'Explorateur ? Remarquez-vous un effet sur la Flam ?
8. **En cas de problème grave.** Si la Flam redémarre en boucle ou ne démarre plus, n'envoyez
   plus rien, appliquez le « Dernier recours » de la section 2 et ouvrez une issue.

## 4. Remonter votre retour

Quel que soit le résultat (réussite comprise : un « tout est OK » est précieux), ouvrez une
issue sur GitHub avec le modèle **« Retour testeur Flam »** :

1. Dans « Ma Flam », rubrique **Retour de test** (en bas de l'écran), cliquez sur **Copier le
   diagnostic**. Le message « Diagnostic copié » confirme la copie. Si la copie est refusée, le
   texte s'affiche dans une zone sélectionnable : cliquez dedans, **Ctrl + A** puis **Ctrl + C**.
   Le bouton fonctionne aussi sans Flam détectée et quand l'écriture sur la Flam est bloquée.
2. Ouvrez le modèle avec le bouton **Signaler un problème**, juste à côté (il s'ouvre dans votre
   navigateur ; sinon **Copier le lien**), ou directement :
   <https://github.com/scarlaty/flam-player/issues/new?template=retour-testeur-flam.yml> (un
   compte GitHub est nécessaire).
3. Remplissez-le : modèle de Flam, ligne « Firmware » de la carte « Flam détectée », version du
   launcher (écran « Paramètres », rubrique « À propos »), numéro du test, résultat attendu et
   obtenu, messages affichés à l'écran. Collez le diagnostic tel quel dans le champ
   « Diagnostic ».
4. Ne publiez **jamais** de clé, de fichier `.mdf`, de numéro de série ni de contenu d'histoire
   officielle.

Le diagnostic est lu sur la Flam sans rien y écrire, et sans accès à internet. Il contient la
version du launcher et de Windows, l'état du firmware et du fichier `.mdf` (taille, en-tête,
disposition, sans les clés), l'état des index (nombres d'entrées, fins de ligne, anomalies), la
présence des dossiers et fichiers de travail, l'espace libre, le nombre d'histoires et les
dernières erreurs de transfert. Il ne contient ni clé, ni numéro de série, ni titre ou
identifiant d'histoire, ni votre nom d'utilisateur (le dossier de votre profil Windows y est
remplacé par `%USERPROFILE%`). Relisez-le avant de le publier.

## En cas de problème

Voir le [dépannage](flam-transfert.md#dépannage). En cas de doute, **ne continuez pas** :
restaurez votre copie (section « Restaurer ») puis signalez le problème.
