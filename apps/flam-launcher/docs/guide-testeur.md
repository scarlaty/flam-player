# Guide testeur : flam-launcher et Flam en firmware 2.x

Merci de tester ! Cette pré-version envoie des histoires sur une Flam branchée en USB. Sur les
Flam en **firmware 1.x**, la fonction a été validée sur un appareil réel ; sur les Flam en
**firmware 2.x**, elle est **expérimentale et n'a jamais été essayée sur un appareil** : c'est
justement ce que vous allez tester. Elle est **non officielle** : lisez
[les risques](flam-transfert.md#risques) avant de commencer.

Vous n'avez pas besoin d'être développeur. Comptez environ une heure.

## 1. Installer la pré-version

1. Téléchargez l'installateur `Flam Launcher_<version>_x64-setup.exe` depuis la page de la
   pré-version sur GitHub, ainsi que la valeur **SHA-256** indiquée dans ses notes.
2. **Vérifiez l'empreinte SHA-256** (cela prouve que le fichier n'a pas été altéré). Ouvrez
   PowerShell dans le dossier de téléchargement et tapez :

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
2. Dans l'Explorateur, affichez les éléments cachés : menu **Afficher**, puis **Afficher**,
   puis cochez **Éléments masqués**. (Certains fichiers de la Flam, comme `.mdf`, peuvent être
   invisibles autrement.)
3. Créez un dossier sur votre PC, par exemple `C:\Sauvegarde-Flam-2026-10-20`.
4. Ouvrez le disque de la Flam, appuyez sur **Ctrl + A** (tout sélectionner), puis **Ctrl + C**
   (copier). Ouvrez votre dossier de sauvegarde et appuyez sur **Ctrl + V** (coller).
   La copie peut durer plusieurs minutes : attendez la fin.
5. **Vérifiez la copie** : clic droit sur le disque de la Flam, puis **Propriétés** ; faites la
   même chose sur votre dossier de sauvegarde. Le nombre de fichiers et de dossiers et la taille
   doivent être les mêmes (à quelques octets près). Vérifiez que vous voyez bien dans la copie
   les dossiers `str` et `etc` et le fichier `.mdf`.
6. Gardez ce dossier jusqu'à la fin de la bêta. Ne le modifiez pas.

### Restaurer

<a id="restaurer"></a>

Si la Flam ne fonctionne plus correctement après un test :

1. Fermez le launcher. Branchez la Flam, mode connexion USB activé.
2. Ouvrez votre dossier de sauvegarde, **Ctrl + A**, **Ctrl + C**, puis ouvrez le disque de la
   Flam et faites **Ctrl + V**. Si Windows demande quoi faire des fichiers existants, choisissez
   **Remplacer les fichiers dans la destination**.
3. Pour revenir exactement à l'état sauvegardé, supprimez aussi sur la Flam les dossiers
   d'histoires que votre copie ne contient pas (dans le dossier `str`).
4. Éjectez la Flam, débranchez-la, puis redémarrez-la.

Ne **formatez** jamais la Flam.

## 3. Les tests

Notez pour chaque test ce que vous voyez, surtout si ce n'est pas ce qui est attendu. Une
seule histoire courte suffit pour les tests 3 à 6.

1. **Détection.** Branchez la Flam (mode connexion USB), ouvrez le launcher, onglet **Ma
   Flam**. Attendu : le bandeau « Fonction expérimentale et non officielle » et, pour une Flam
   en 2.x, « firmware 2.x : en test, sauvegardez votre Flam avant » ; la carte « Flam
   détectée » avec le disque et l'espace libre. Débranchez puis rebranchez la Flam, puis
   cliquez sur **Réessayer** : la Flam doit être de nouveau détectée.
2. **Ma Flam : titres et vignettes.** Attendu : la liste de vos histoires avec leur titre et
   leur image. Une histoire sans titre est affichée « Histoire » suivie d'un identifiant : notez
   combien et lesquelles. Notez aussi tout avertissement « À noter » ou sur l'index.
3. **Envoyer une histoire.** Dans « Mes histoires » (installez d'abord une histoire depuis une
   source si besoin), cliquez sur **Envoyer sur la Flam**, lisez l'avertissement et confirmez.
   Attendu : une progression (copie, puis vérification) jusqu'à « Envoi terminé ». **Ne
   débranchez pas la Flam pendant l'envoi.** Notez la durée et la taille de l'histoire.
4. **La lancer sur la Flam.** Éjectez la Flam (test 7), débranchez-la, **redémarrez-la**, puis
   cherchez l'histoire dans la liste de la Flam et lancez-la. Attendu : titre lisible, images,
   sons, histoire jouable jusqu'au bout. Notez tout écran noir, titre illisible ou blocage.
5. **Retirer.** Rebranchez la Flam, onglet **Ma Flam**, cliquez sur **Retirer** à côté de
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
   est nécessaire) et choisissez le modèle **« Retour testeur Flam »** (modèle en cours de
   création, #318).
2. Remplissez le modèle : modèle de Flam, version du firmware, numéro de test, ce que vous avez
   vu.
3. Dans « Ma Flam », cliquez sur **Copier le diagnostic** (bouton en cours de création, #318)
   et collez le résultat dans l'issue. Il aide à comprendre ce qui s'est passé.
4. Ne publiez **jamais** de clé, de fichier `.mdf` ni de contenu d'histoire officielle.

Si le modèle ou le bouton ne sont pas encore disponibles dans votre version, décrivez votre
retour avec vos mots : la version du launcher (écran « Paramètres », rubrique à propos), le firmware de la
Flam et les messages affichés à l'écran.

## En cas de problème

Voir le [dépannage](flam-transfert.md#dépannage). En cas de doute, **ne continuez pas** :
restaurez votre copie (section « Restaurer ») puis signalez le problème.
