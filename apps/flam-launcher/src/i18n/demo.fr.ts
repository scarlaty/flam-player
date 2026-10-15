/**
 * Textes de la page de démonstration (développement uniquement). Séparés de `fr.ts`
 * pour ne pas être embarqués dans l'application publiée.
 */
export const demoFr = {
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
  openAlertDialog: "Confirmer une suppression",
  dialogTitle: "Installer l'histoire ?",
  dialogDescription:
    "L'histoire sera téléchargée, convertie puis ajoutée à la bibliothèque.",
  dialogBody: "Vous pourrez suivre l'avancement dans le panneau des tâches.",
  alertTitle: "Supprimer la source ?",
  alertDescription:
    "La source disparaîtra de la liste. Les histoires déjà installées sont conservées.",
  rename: (label: string) => `Renommer : ${label}`,
  moveUp: (label: string) => `Monter : ${label}`,
  deleteSource: (label: string) => `Supprimer : ${label}`,
  converting: "Conversion…",
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
};
