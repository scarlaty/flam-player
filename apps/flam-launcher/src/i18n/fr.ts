/**
 * Textes de l'interface en français. Tout texte affiché passe par ce fichier pour
 * préparer une future traduction : une autre langue fournira un objet du même type.
 */
export const fr = {
  app: {
    name: "Flam Launcher",
    skipToContent: "Aller au contenu",
    sourcesNav: "Sources d'histoires",
    demoLink: "Démonstration des composants (développement)",
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
};

/** Forme commune à toutes les langues. */
export type Messages = typeof fr;
