import { invokeCommand } from "./error";

/**
 * Ouvre une page du dépôt du projet dans le navigateur par défaut (commande `open_external`).
 * Le backend n'accepte que `https://github.com/scarlaty/flam-player/…` : toute autre adresse est
 * refusée (`AppError` de kind `validation`). Aucune permission du plugin d'ouverture n'est donnée
 * au front, cette commande est le seul passage.
 */
export async function openExternal(url: string): Promise<void> {
  await invokeCommand<void>("open_external", { url });
}
