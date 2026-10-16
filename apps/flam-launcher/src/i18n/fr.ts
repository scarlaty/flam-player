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
  },
  errors: {
    title: (kind: string) =>
      ({
        validation: "Valeur refusée",
        not_found: "Élément introuvable",
        io: "Erreur d'accès aux fichiers",
        config_version: "Configuration trop récente",
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
    themeFailed: "Le thème n'a pas pu être enregistré.",
    about: {
      title: "À propos",
      version: (name: string, version: string) => `${name} — version ${version}`,
      license: "Licence MIT.",
      credits:
        "Inspiré de l'ergonomie de Telmi Store et de TelmiSync. Aucun code ni ressource de ces projets n'a été repris.",
    },
  },
  welcome: {
    title: "Bienvenue",
    version: (name: string, version: string) => `${name} — version ${version}`,
    error: "Impossible de lire les informations de l'application.",
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
    count: (n: number) => (n <= 1 ? `${n} histoire` : `${n} histoires`),
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
  toast: {
    region: "Notifications",
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
