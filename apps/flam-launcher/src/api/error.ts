import { invoke } from "@tauri-apps/api/core";

/**
 * Erreur renvoyée par le backend (miroir de `AppError` dans `src-tauri/src/error.rs`).
 * `kind` est un code stable sur lequel le front peut s'appuyer ; `message` est lisible.
 */
export interface AppError {
  kind: string;
  message: string;
}

export function isAppError(value: unknown): value is AppError {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return typeof candidate.kind === "string" && typeof candidate.message === "string";
}

/** Convertit n'importe quelle erreur en `AppError` (kind "unknown" si forme inattendue). */
export function toAppError(error: unknown): AppError {
  if (isAppError(error)) return error;
  if (typeof error === "string") return { kind: "unknown", message: error };
  if (error instanceof Error) return { kind: "unknown", message: error.message };
  return { kind: "unknown", message: "Erreur inconnue" };
}

/** Appelle une commande Tauri ; toute erreur est normalisée en `AppError`. */
export async function invokeCommand<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw toAppError(error);
  }
}
