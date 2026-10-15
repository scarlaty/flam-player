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
  welcome: {
    title: "Bienvenue",
    version: (name: string, version: string) => `${name} — version ${version}`,
    error: "Impossible de lire les informations de l'application.",
  },
  common: {
    close: "Fermer",
    cancel: "Annuler",
  },
  sources: {
    kind: {
      library: "Bibliothèque",
      catalog: "Catalogue",
      folder: "Dossier local",
    },
    add: "Ajouter une source",
    count: (n: number) => (n <= 1 ? `${n} histoire` : `${n} histoires`),
    unavailable: "Indisponible",
  },
  story: {
    age: (min: number, max?: number) =>
      max === undefined ? `Dès ${min} ans` : `${min}-${max} ans`,
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
    },
    cancel: (title: string) => `Annuler : ${title}`,
    dismiss: (title: string) => `Retirer : ${title}`,
    progress: (title: string) => `Progression : ${title}`,
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
  demo: {
    title: "Démonstration des composants",
    link: "Démonstration des composants (développement)",
    notice:
      "Page réservée au développement : elle n'existe pas dans l'application publiée.",
    buttons: "Boutons",
    badges: "Badges",
    progress: "Barres de progression",
    stories: "Cartes d'histoires",
    empty: "État vide",
    dialogs: "Dialogue et notifications",
    openDialog: "Ouvrir un dialogue",
    showToast: "Afficher une notification",
    showErrorToast: "Notification d'erreur",
    dialogTitle: "Installer l'histoire ?",
    dialogBody:
      "L'histoire sera téléchargée, convertie puis ajoutée à la bibliothèque. Vous pourrez suivre l'avancement dans le panneau des tâches.",
    install: "Installer",
    play: "Jouer",
    refresh: "Actualiser",
    settings: "Paramètres",
    delete: "Supprimer",
    emptyTitle: "Aucune histoire ici",
    emptyBody:
      "Ajoutez un dossier ou un catalogue pour commencer à remplir la bibliothèque.",
    toastDone: "« La forêt des lucioles » est prête à être jouée.",
    toastError: "Le catalogue n'a pas pu être lu. Vérifiez l'adresse et réessayez.",
    progressDownload: "Téléchargement de démonstration",
    progressWaiting: "Attente indéterminée",
  },
};

/** Forme commune à toutes les langues. */
export type Messages = typeof fr;
