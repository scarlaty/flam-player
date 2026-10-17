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
