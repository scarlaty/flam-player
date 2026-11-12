import { open } from "@tauri-apps/plugin-dialog";
import { t } from "../i18n";
import { toAppError } from "./error";

/**
 * Sélecteur natif de dossier (permission `dialog:allow-open`). Renvoie `null` si
 * l'utilisateur annule. Le chemin est ensuite validé par le backend.
 */
export async function pickFolder(title?: string): Promise<string | null> {
  try {
    return await open({ directory: true, multiple: false, title });
  } catch (error) {
    throw toAppError(error);
  }
}

/** Sélecteur natif de fichier `.exe`. Renvoie `null` si l'utilisateur annule. */
export async function pickExecutable(title?: string): Promise<string | null> {
  try {
    return await open({
      directory: false,
      multiple: false,
      title,
      filters: [{ name: t.common.executableFilter, extensions: ["exe"] }],
    });
  } catch (error) {
    throw toAppError(error);
  }
}

/** Sélecteur natif d'un fichier quelconque. Renvoie `null` si l'utilisateur annule. */
export async function pickFile(title?: string): Promise<string | null> {
  try {
    return await open({ directory: false, multiple: false, title });
  } catch (error) {
    throw toAppError(error);
  }
}
