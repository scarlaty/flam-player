const storyCount = (n: number) => (n <= 1 ? `${n} histoire` : `${n} histoires`);

/**
 * Textes de l'interface en français. Tout texte affiché passe par ce fichier pour
 * préparer une future traduction : une autre langue fournira un objet du même type.
 */
export const fr = {
  app: {
    name: "Flam Launcher",
    skipToContent: "Aller au contenu",
    sourcesNav: "Sources d'histoires",
  },
  nav: {
    label: "Navigation",
    home: "Accueil",
    sources: "Sources",
    settings: "Paramètres",
    library: "Bibliothèque",
    device: "Ma Flam",
  },
  errors: {
    title: (kind: string) =>
      ({
        validation: "Valeur refusée",
        not_found: "Élément introuvable",
        io: "Erreur d'accès aux fichiers",
        config_version: "Configuration trop récente",
        network: "Réseau indisponible",
        catalog_invalid: "Catalogue illisible",
        folder_unavailable: "Dossier introuvable",
      })[kind] ?? "Une erreur est survenue",
    loadFailed: "Impossible de charger les données.",
    retry: "Réessayer",
  },
  recovery: {
    title: (file: string) => `Le fichier ${file} était illisible`,
    body: (backup: string) =>
      `Il a été remplacé par les valeurs par défaut. L'ancien contenu est conservé dans « ${backup} », à côté du fichier, dans le dossier de configuration.`,
    ack: "J'ai compris",
    ackFailed: "Impossible d'enregistrer l'acquittement.",
  },
  sourcesScreen: {
    intro:
      "Les sources fournissent les histoires affichées dans la bibliothèque. Faites glisser une ligne ou utilisez les boutons Monter et Descendre pour changer l'ordre.",
    addCatalog: "Ajouter un catalogue",
    addFolder: "Ajouter un dossier",
    pickFolderTitle: "Choisir un dossier d'histoires",
    defaultFolderName: "Dossier d'histoires",
    listLabel: "Sources enregistrées",
    libraryNote: "Toujours en première position",
    moveUp: (name: string) => `Monter « ${name} »`,
    moveDown: (name: string) => `Descendre « ${name} »`,
    rename: (name: string) => `Renommer « ${name} »`,
    remove: (name: string) => `Supprimer « ${name} »`,
    moved: (name: string, position: number, total: number) =>
      `« ${name} » : position ${position} sur ${total}`,
    added: (name: string) => `Source « ${name} » ajoutée`,
    renamed: (name: string) => `Source renommée en « ${name} »`,
    removed: (name: string) => `Source « ${name} » supprimée`,
    catalogDialog: {
      title: "Ajouter un catalogue",
      description:
        "Un catalogue est une adresse web qui liste des histoires (format JSON).",
      name: "Nom",
      url: "Adresse du catalogue",
      urlHint: "Exemple : https://exemple.org/catalogue.json",
      submit: "Ajouter",
    },
    renameDialog: {
      title: "Renommer la source",
      name: "Nom",
      submit: "Renommer",
    },
    deleteDialog: {
      title: "Supprimer cette source ?",
      description: (name: string) =>
        `« ${name} » sera retirée de la liste. Aucun fichier de votre ordinateur n'est supprimé.`,
      confirm: "Supprimer",
    },
  },
  validation: {
    nameEmpty: "Le nom ne peut pas être vide.",
    nameControl: "Le nom ne peut pas contenir de caractères de contrôle.",
    nameTooLong: (max: number) => `Le nom ne peut pas dépasser ${max} caractères.`,
    urlEmpty: "L'adresse ne peut pas être vide.",
    urlTooLong: (max: number) => `L'adresse ne peut pas dépasser ${max} caractères.`,
    urlInvalid: "L'adresse doit commencer par http:// ou https:// et contenir un site.",
    urlCredentials:
      "L'adresse ne doit pas contenir d'identifiants (utilisateur:mot de passe@).",
    pathNotAbsolute: "Indiquez un chemin complet, par exemple C:\\Dossier.",
    pathNetwork: "Les chemins réseau ne sont pas acceptés.",
    pathNotExe: "Le fichier doit se terminer par .exe.",
  },
  settingsScreen: {
    paths: "Programmes et dossiers",
    save: "Enregistrer",
    saved: "Paramètres enregistrés",
    automatic: "Automatique",
    browse: "Parcourir…",
    autoPlaceholder: "Automatique",
    issuesTitle: "Points à vérifier",
    player: "Chemin de flam-player",
    python: "Chemin de Python",
    pythonNote:
      "Le convertisseur livré avec l'application embarque son propre Python : le chemin de Python ci-dessous n'est utilisé que si ce convertisseur est absent (par exemple en développement).",
    cacheDir: "Dossier de cache",
    pickPlayer: "Choisir flam-player.exe",
    pickPython: "Choisir python.exe",
    pickCache: "Choisir le dossier de cache",
    used: (path: string) => `Utilisé : ${path}`,
    notFound: "Aucun fichier trouvé : choisissez-le avec « Parcourir… ».",
    cache: {
      title: "Cache",
      usage: (size: string, path: string) => `${size} occupés dans ${path}`,
      empty: (path: string) => `Le cache est vide (${path}).`,
      clear: "Vider le cache",
      confirmTitle: "Vider le cache ?",
      confirmDescription: (dirs: string, path: string, size: string) =>
        `Seuls les dossiers créés par Flam Launcher (${dirs}) seront supprimés dans « ${path} ». Le reste de ce dossier n'est pas touché. Espace libéré : ${size}.`,
      cleared: (size: string) => `Cache vidé : ${size} libérés`,
    },
    appearance: "Apparence",
    playback: {
      title: "Lecture",
      minimize: "Réduire la fenêtre pendant la lecture",
      minimizeHint:
        "Quand flam-player s'ouvre dans une fenêtre séparée, la fenêtre du launcher est réduite à son lancement, puis rétablie à la fin.",
    },
    themeFailed: "Le thème n'a pas pu être enregistré.",
    thumbFit: {
      title: "Vignettes",
      label: "Vignettes : remplissage",
      note: "Les vignettes de bibliothèque des histoires converties sont en 104 × 136 (portrait). Ce réglage s'applique aux prochaines conversions ; les histoires déjà converties gardent leur vignette.",
      failed: "Le remplissage des vignettes n'a pas pu être enregistré.",
      options: {
        flou: {
          label: "Flou",
          hint: "Image entière, fond rempli par la même image agrandie et floutée.",
        },
        rogner: {
          label: "Rogner",
          hint: "Image agrandie et rognée au centre pour remplir la vignette.",
        },
        bandes: {
          label: "Bandes",
          hint: "Image entière, bandes transparentes en haut et en bas.",
        },
      },
    },
    about: {
      title: "À propos",
      version: (name: string, version: string) => `${name} — version ${version}`,
      license: "Licence GPL v3.",
    },
  },
  welcome: {
    title: "Bienvenue",
    version: (name: string, version: string) => `${name} — version ${version}`,
    error: "Impossible de lire les informations de l'application.",
  },
  library: {
    sourceNotFound: {
      title: "Source introuvable",
      description: "Cette source n'existe plus. Choisissez-en une dans la liste.",
      action: "Aller à la première source",
    },
    refresh: "Actualiser",
    refreshing: "Actualisation…",
    loading: "Chargement des histoires…",
    loadFailed: "Impossible de charger les histoires de cette source.",
    refreshed: (name: string) => `« ${name} » est à jour`,
    refreshedOffline: (name: string) =>
      `« ${name} » est injoignable : les données enregistrées sont affichées.`,
    offlineTitle: "Hors ligne",
    offlineBody: (when: string | null) =>
      when
        ? `Le catalogue est injoignable. Les histoires affichées viennent de la dernière vérification, le ${when}.`
        : "Le catalogue est injoignable. Les histoires affichées viennent des données enregistrées.",
    problemTitle: "Le dernier rafraîchissement a échoué",
    warningsTitle: (n: number) =>
      n <= 1 ? `${n} élément ignoré` : `${n} éléments ignorés`,
    warningsHint: "Ces éléments sont illisibles ou invalides.",
    filtersLabel: "Recherche et filtres",
    search: "Rechercher une histoire",
    searchPlaceholder: "Titre, auteur, catégorie…",
    age: "Âge de l'enfant",
    allAges: "Tous les âges",
    ageOption: (age: number) => `${age} ans`,
    category: "Catégorie",
    allCategories: "Toutes les catégories",
    sort: "Trier par",
    sortOptions: {
      title: "Titre (A-Z)",
      recent: "Plus récentes",
      popular: "Popularité",
    },
    resultCount: (shown: number, total: number) =>
      shown === total ? storyCount(total) : `${storyCount(shown)} sur ${total}`,
    gridLabel: (name: string) => `Histoires de ${name}`,
    showMore: (remaining: number) => `Afficher plus (${remaining} restantes)`,
    resetFilters: "Réinitialiser les filtres",
    empty: {
      library: {
        title: "Aucune histoire installée",
        description:
          "Les histoires installées apparaîtront ici. Parcourez un catalogue ou un dossier pour en ajouter.",
        action: "Gérer les sources",
      },
      catalog: {
        title: "Ce catalogue est vide",
        description: "Il ne propose aucune histoire pour le moment.",
      },
      folder: {
        title: "Aucune histoire dans ce dossier",
        description:
          "Ajoutez des archives .zip, des dossiers .plain ou des fichiers .plain.pk dans le dossier.",
      },
      noResults: {
        title: "Aucun résultat",
        description: "Aucune histoire ne correspond à la recherche et aux filtres.",
      },
    },
    state: {
      to_download: "À télécharger",
      to_convert: "À convertir",
      to_install: "À installer",
      installed: "Installée",
      update_available: "Mise à jour disponible",
      in_progress: "En cours",
      incompatible: "Incompatible",
      untested: "Non testée",
    },
  },
  compat: {
    level: {
      compatible: "Compatible",
      warnings: "Avertissements",
      incompatible: "Incompatible",
      untested: "Non testée",
    },
    /** Raison affichée quand la fiche n'en fournit pas. */
    defaultReason: {
      compatible: "Aucun problème constaté lors des essais.",
      warnings: "Jouable, mais des limites ont été constatées lors des essais.",
      incompatible: "Cette histoire a échoué lors des essais de compatibilité.",
      untested: "Pas encore évaluée.",
    },
    /** Raison propre à chaque statut d'échec connu de la campagne. */
    failureReason: {
      ECHEC_TELECHARGEMENT: "Le téléchargement a échoué lors des essais.",
      ECHEC_CONVERSION: "La conversion a échoué lors des essais.",
      ECHEC_VALIDATION: "L'histoire convertie n'est pas valide.",
      ECHEC_EXECUTION: "L'histoire s'arrête ou se bloque lors de son exécution.",
    } as Record<string, string>,
    stale: "Résultat périmé : évalué avec une autre version du convertisseur.",
    issue: {
      title: "Rapport de compatibilité indisponible",
      unreadable:
        "Le fichier de statuts est illisible : les histoires apparaissent non testées.",
      tooLarge: "Le fichier de statuts est trop volumineux : il n'est pas lu.",
      unsupportedVersion: (found: number | null, supported: number) =>
        `Le fichier de statuts utilise un format ${found === null ? "inconnu" : `n° ${found}`} (pris en charge : n° ${supported}) : mettez à jour l'application.`,
    },
    install: {
      title: (story: string) => `Installer « ${story} » malgré tout ?`,
      description: "Cette histoire est signalée comme incompatible.",
      body: "Elle risque de ne pas se convertir ou de ne pas fonctionner correctement dans le player. Vous pouvez l'installer quand même.",
      confirm: "Installer quand même",
      cancel: "Annuler",
    },
  },
  common: {
    close: "Fermer",
    cancel: "Annuler",
    loading: "Chargement…",
  },
  sources: {
    kind: {
      library: "Bibliothèque",
      catalog: "Catalogue",
      folder: "Dossier local",
    },
    title: "Sources",
    add: "Ajouter une source",
    count: storyCount,
    status: {
      offline: "Hors ligne",
      error: "Erreur",
    },
  },
  story: {
    age: (min: number, max?: number) =>
      max === undefined ? `Dès ${min} ans` : `${min}-${max} ans`,
    progress: (title: string) => `Progression : ${title}`,
  },
  status: {
    installed: "Installée",
    available: "Disponible",
    compatible: "Compatible",
    partial: "Compatibilité partielle",
    incompatible: "Incompatible",
    new: "Nouveau",
  },
  tasks: {
    title: "Tâches en cours",
    empty: "Aucune tâche en cours.",
    kind: {
      download: "Téléchargement",
      convert: "Conversion",
      install: "Installation",
    },
    state: {
      queued: "En attente",
      running: "En cours",
      done: "Terminée",
      error: "Échec",
      cancelled: "Annulée",
    },
    cancel: (title: string) => `Annuler : ${title}`,
    retry: (title: string) => `Réessayer : ${title}`,
    dismiss: (title: string) => `Retirer : ${title}`,
    progress: (title: string) => `Progression : ${title}`,
    /** Annonce d'un changement d'état, lue par les lecteurs d'écran. */
    announce: (title: string, kind: string, state: string, detail?: string) =>
      `${kind} « ${title} » : ${state}${detail ? `. ${detail}` : ""}`,
  },
  stories: {
    detail: {
      cover: (title: string) => `Vignette de « ${title} »`,
      noDescription: "Aucune description.",
      facts: {
        age: "Âge",
        category: "Catégorie",
        author: "Auteur",
        voice: "Voix",
        license: "Licence",
        size: "Taille",
        source: "Source",
      },
      badges: "Statut de l'histoire",
      unknown: "Non précisé",
    },
    actions: {
      downloadAndPlay: "Télécharger et jouer",
      download: "Télécharger",
      installAndPlay: "Installer et jouer",
      install: "Installer",
      play: "Jouer",
      playFromFolder: "Jouer depuis le dossier",
      update: "Mettre à jour",
      remove: "Supprimer",
      cancel: "Annuler",
      /** Libellé accessible des actions rapides d'une carte. */
      forStory: (action: string, title: string) => `${action} « ${title} »`,
    },
    progress: {
      queued: "En attente",
      downloading: "Téléchargement",
      downloadingSize: (received: string, total: string) =>
        `Téléchargement : ${received} sur ${total}`,
      converting: "Conversion",
      installing: "Installation",
      phase: {
        images: "images",
        audio: "audio",
        scripts: "scripts",
        zip: "archive",
      } as Record<string, string>,
      convertingPhase: (phase: string) => `Conversion : ${phase}`,
    },
    playing: "En cours de lecture dans flam-player",
    playerBusy:
      "Une histoire est déjà en cours de lecture : fermez flam-player pour jouer.",
    removeDialog: {
      title: (title: string) => `Supprimer « ${title} » ?`,
      description:
        "L'histoire est retirée de « Mes histoires » et son fichier converti est supprimé. Vous pourrez la réinstaller depuis sa source.",
      confirm: "Supprimer",
      purgeSaves: "Supprimer aussi la progression",
      purgeSavesHint:
        "Efface les sauvegardes de l'enfant. Cette action est irréversible. Décochée, la progression est conservée et retrouvée si l'histoire est réinstallée.",
    },
    toast: {
      entryGone: (title: string) =>
        `« ${title} » n'est plus proposée par sa source (catalogue ou dossier modifié). Rafraîchissez la source, puis relancez l'installation depuis la fiche.`,
      savesLost: (title: string, detail: string) =>
        `La progression de « ${title} » n'a pas pu être conservée telle quelle : ${detail}`,
      installed: (title: string) => `« ${title} » est installée`,
      installedWarnings: (title: string, count: number) =>
        `« ${title} » est installée (${count <= 1 ? `${count} avertissement` : `${count} avertissements`} à la conversion)`,
      removed: (title: string) => `« ${title} » a été supprimée`,
      finished: (title: string) => `Lecture de « ${title} » terminée`,
      playerFailed: (title: string, detail: string) =>
        `flam-player s'est arrêté sur une erreur pendant « ${title} ».${detail ? ` ${detail}` : ""}`,
    },
  },
  deviceScreen: {
    intro:
      "Les histoires installées sur votre Flam. Vous pouvez en envoyer depuis « Mes histoires » ou en retirer ; la Flam n'est modifiée que sur votre demande.",
    searching: "Recherche de la Flam…",
    notDetected: {
      title: "Aucune Flam détectée",
      description: "Activez le mode connexion USB sur la Flam",
      retry: "Réessayer",
    },
    announce: {
      detected: "Flam détectée",
      lost: "Flam débranchée ou mode connexion USB désactivé",
      stories: (count: number) => `${storyCount(count)} sur la Flam`,
    },
    detected: {
      title: "Flam détectée",
      volume: "Volume",
      fileSystem: "Système de fichiers",
      unnamedVolume: "(sans nom)",
      several: (count: number) =>
        `${count} Flam sont branchées. Seule la première est affichée ; débranchez les autres.`,
    },
    space: {
      label: "Espace utilisé sur la Flam",
      summary: (free: string, total: string) => `${free} libres sur ${total}`,
      unavailable: "Espace disque indisponible",
    },
    warnings: {
      title: "À noter",
      missingStr: "Le dossier des histoires (str) est introuvable sur la Flam.",
      missingLibrary: "Le dossier de l'index (etc/library) est introuvable sur la Flam.",
      unexpectedMdf: (size: number) =>
        `Le fichier .mdf de la Flam a une taille inattendue (${size} octets).`,
      mdfTruncated: "Le fichier .mdf de la Flam est tronqué ou vide.",
      mdfTooLarge: "Le fichier .mdf de la Flam est démesuré : il n'a pas été lu.",
      backupFailed:
        "La copie de sauvegarde du fichier .mdf n'a pas pu être enregistrée sur cet ordinateur.",
    },
    stories: {
      title: (count: number) => `Histoires sur la Flam (${count})`,
      listLabel: "Histoires installées sur la Flam",
      loading: "Lecture des histoires…",
      refresh: "Actualiser la liste",
      untitled: (shortId: string) => `Histoire ${shortId}`,
      idDetail: (shortId: string) => `Identifiant ${shortId}`,
      hidden: "Masquée",
      unlisted: "Non listée",
      missing: "Dossier manquant",
      emptyTitle: "Aucune histoire sur la Flam",
      emptyDescription: "Les histoires envoyées depuis le launcher apparaîtront ici.",
      loadFailed: "Impossible de lire les histoires de la Flam.",
      noAnswer: "La Flam n'a pas répondu : vérifiez la connexion USB.",
    },
    index: {
      visibleAbsent: "La Flam n'a pas d'index d'histoires (etc/library/list).",
      visibleDamaged:
        "L'index des histoires de la Flam contient des lignes invalides ou en double : elles sont ignorées.",
      visibleUnreadable:
        "L'index des histoires de la Flam est illisible. Les dossiers trouvés sont affichés comme non listés.",
      hiddenDamaged:
        "L'index des histoires masquées contient des lignes invalides ou en double : elles sont ignorées.",
      hiddenUnreadable: "L'index des histoires masquées est illisible.",
      truncated: (limit: number) =>
        `La liste est tronquée : seules les ${limit} premières histoires sont affichées.`,
      strUnreadable:
        "Le dossier des histoires (str) est illisible : seules les histoires de l'index sont affichées.",
    },
    order: {
      edit: "Modifier l'ordre",
      busy: "Ordre non modifiable pendant un envoi.",
      title: "Ordre des histoires sur la Flam",
      help: "Glissez les histoires, ou utilisez les boutons Monter et Descendre, puis enregistrez. La Flam affiche les histoires dans cet ordre.",
      othersNote:
        "Les histoires masquées ou non listées ne font pas partie de l'ordre et ne bougent pas.",
      listLabel: "Ordre des histoires visibles",
      position: (position: number) => `${position}.`,
      moveUp: (name: string) => `Monter « ${name} »`,
      moveDown: (name: string) => `Descendre « ${name} »`,
      moved: (name: string, position: number, total: number) =>
        `« ${name} » déplacée en position ${position} sur ${total}`,
      save: "Enregistrer l'ordre",
      saving: "Enregistrement de l'ordre…",
      cancel: "Annuler",
      unchangedHint: "Déplacez au moins une histoire pour enregistrer un nouvel ordre.",
      reset: "La liste de la Flam a changé : l'ordre a été réinitialisé.",
      saved:
        "Ordre enregistré sur la Flam. Redémarrez la Flam pour qu'elle affiche le nouvel ordre.",
      unchanged: "L'ordre de la Flam était déjà celui-ci : rien n'a été écrit.",
      failed: (message: string) => `Impossible d'enregistrer l'ordre : ${message}`,
    },
  },
  deviceTransfer: {
    send: "Envoyer sur la Flam",
    replace: "Remplacer sur la Flam",
    /** La copie de bibliothèque est plus ancienne que sa source dossier (#288). */
    staleCopy: "Source modifiée : mettez l'histoire à jour avant l'envoi",
    forStory: (action: string, title: string) => `${action} « ${title} »`,
    reasons: {
      noDevice: "Flam non détectée",
      present: "Déjà sur la Flam",
      noSpace: "Espace insuffisant sur la Flam",
      firmware: "Firmware de la Flam non pris en charge",
      queued: "Dans la file d'envoi",
      checking: "Lecture de la Flam…",
    },
    experimental: {
      title: "Fonction expérimentale, non officielle",
      description:
        "L'envoi d'histoires sur la Flam n'est ni prévu ni garanti par son fabricant.",
      body: "Le launcher écrit directement sur la mémoire de la Flam. Il vérifie chaque copie avant de l'installer et ne touche à rien en cas d'échec, mais gardez une copie de vos histoires. Ne débranchez pas la Flam pendant un envoi.",
      confirm: "J'ai compris, envoyer",
    },
    panel: {
      title: "Envoi sur la Flam",
      overall: (done: number, total: number) => `Histoire ${done} sur ${total}`,
      overallLabel: "Progression de l'envoi",
      current: (title: string) => `En cours : ${title}`,
      currentLabel: (title: string) => `Progression de « ${title} »`,
      phase: { copy: "Copie sur la Flam", verify: "Vérification de la copie" },
      estimate: (minutes: number) =>
        minutes <= 1
          ? "Environ 1 minute restante"
          : `Environ ${minutes} minutes restantes`,
      cancel: "Annuler l'envoi",
      cancelling: "Annulation en cours…",
      removeFromQueue: (title: string) => `Retirer « ${title} » de la file`,
      dismiss: "Fermer la liste",
      listLabel: "File d'envoi",
      state: {
        waiting: "En attente",
        sending: "Envoi en cours",
        done: "Envoyée",
        failed: "Échec",
        cancelled: "Annulée",
      },
      announce: {
        started: (title: string) => `Envoi de « ${title} » commencé`,
        done: (title: string) => `« ${title} » envoyée sur la Flam`,
        failed: (title: string, message: string) =>
          `Échec de l'envoi de « ${title} » : ${message}`,
        cancelled: "Envoi annulé",
        finished: "Envoi terminé",
      },
    },
    finished: {
      title: "Envoi terminé",
      restart:
        "Redémarrez la Flam pour qu'elle reconstruise sa bibliothèque : les nouvelles histoires apparaîtront ensuite.",
    },
    errors: {
      noDevice: "La Flam n'est plus détectée : rien n'a été envoyé.",
      firmware:
        "Le firmware de cette Flam n'est pas pris en charge : l'envoi est impossible.",
      readback:
        "Histoire copiée, mais son titre relu sur la Flam ne se déchiffre pas avec les clés choisies : elle risque d'être illisible. Retirez-la et choisissez une autre histoire officielle.",
    },
    carrier: {
      title: "Flam en firmware 1.x : mode carrier",
      warningTitle: "Avertissement",
      understood: "J'ai compris",
      continue: "Continuer",
      accepting: "Enregistrement…",
      acceptedAt: (date: string) => `Avertissement accepté le ${date}.`,
      loading: "Lecture de la Flam…",
      loadFailed: (message: string) => `Impossible de lire la Flam : ${message}`,
      sourceLegend: "Clés à utiliser",
      stored: "Clés conservées sur ce PC pour cette Flam",
      fromStory: (title: string) => `Histoire officielle « ${title} »`,
      hiddenHint: "Histoire masquée",
      withStoredHint: "Avec la clé conservée sur ce PC",
      fromEntered: "Saisir la clé (bt)",
      fromEnteredHint:
        "La clé bt d'une histoire officielle de cette Flam, en hexadécimal. flam-launcher retrouve l'histoire qu'elle déchiffre et vérifie tout avant la copie.",
      enteredLabel: "Clé bt",
      enteredHint:
        "64 caractères hexadécimaux (0 à 9, a à f) ; espaces et majuscules acceptés.",
      showEntered: "Afficher la clé",
      checkEntered: "Vérifier la clé",
      checkingEntered: "Recherche de l'histoire…",
      recognized: (title: string) =>
        `Clé reconnue : histoire « ${title} ». Vérifiez que le titre est exact : un titre altéré signale une faute de frappe.`,
      titleConfirmed: "Le titre est exact",
      enteredNotHex: "Seuls les chiffres 0 à 9 et les lettres a à f sont acceptés.",
      enteredLength: (count: number, expected: number) =>
        `${count} caractères hexadécimaux sur ${expected} attendus.`,
      fromFile: "Choisir un fichier…",
      fromFileHint:
        "Fichier bt d'une copie de dossier d'histoire officielle (avec key à côté), ou archive zip qui les contient.",
      pickTitle: "Fichier bt ou archive zip",
      pick: "Parcourir…",
      picked: (path: string) => `Fichier choisi : ${path}`,
      noCandidate:
        "Aucune histoire de cette Flam ne contient de fichier bt : saisissez la clé ou choisissez un fichier.",
      remember: "Conserver ces clés sur ce PC",
      rememberHint:
        "Chiffrées pour votre compte Windows et liées à cette Flam. « Oublier les clés » les supprime.",
      confirm: "Vérifier et envoyer",
      forget: "Oublier les clés",
      forgetting: "Suppression…",
      forgotten: "Clés de cette Flam oubliées sur ce PC.",
      nothingToForget: "Aucune clé conservée pour cette Flam.",
      forgetFailed: (message: string) => `Impossible d'oublier les clés : ${message}`,
    },
    recovery: {
      done: "Un transfert interrompu a été repris : la Flam est de nouveau dans un état cohérent.",
      failed: (message: string) =>
        `La reprise d'un éventuel transfert interrompu a échoué : ${message}`,
    },
    fromLibrary: {
      title: "Envoyer depuis « Mes histoires »",
      listLabel: "Histoires de « Mes histoires » à envoyer",
      empty: "Aucune histoire n'est installée dans « Mes histoires ».",
      loadFailed: "Impossible de lire « Mes histoires ».",
    },
    remove: {
      button: "Retirer",
      title: (title: string) => `Retirer « ${title} » de la Flam ?`,
      description:
        "L'histoire sera supprimée de la Flam. Elle reste dans « Mes histoires » si elle y figure.",
      confirm: "Retirer de la Flam",
      done: (title: string) => `« ${title} » retirée de la Flam.`,
      cleanupPending: " L'espace sera libéré à la prochaine opération.",
      failed: (message: string) => `Impossible de retirer l'histoire : ${message}`,
      busy: "Retrait indisponible pendant un envoi.",
      working: "Retrait…",
      otherWorking: "Un retrait est déjà en cours.",
      ejecting: "Retrait indisponible pendant une éjection.",
    },
    eject: {
      button: "Éjecter la Flam",
      working: "Éjection en cours…",
      removing: "Éjection indisponible pendant un retrait.",
      busy: "Éjection indisponible pendant un envoi.",
      failed: (message: string) => `Impossible d'éjecter la Flam : ${message}`,
    },
  },
  toast: {
    region: "Notifications",
  },
  playerScreen: {
    label: "Lecture",
    zone: (title: string) => `Lecteur : ${title}`,
    hint: "Cliquez dans le lecteur ou utilisez « Clavier au lecteur ». Flèches, Entrée, Échap. F6 rend le clavier au launcher.",
    focus: "Clavier au lecteur",
    stop: "Arrêter",
    stopping: "Arrêt en cours…",
    stopFailed: "flam-player n'a pas pu être arrêté.",
    stopTimeout: "flam-player ne répond pas. Réessayez « Arrêter ».",
  },
  theme: {
    label: "Thème",
    system: "Système",
    light: "Clair",
    dark: "Sombre",
  },
};

/** Forme commune à toutes les langues. */
export type Messages = typeof fr;
