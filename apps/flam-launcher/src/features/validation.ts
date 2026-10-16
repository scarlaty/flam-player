import { t } from "../i18n";

/** Limites identiques à celles du backend (`sources.rs`). */
export const MAX_NAME_LEN = 100;
export const MAX_URL_LEN = 2048;

/** Message d'erreur, ou `null` si la valeur est acceptable. */
export type FieldError = string | null;

// eslint-disable-next-line no-control-regex
const CONTROL_CHARS = /[\u0000-\u001f\u007f-\u009f]/;

/** Nom d'une source (même règles que le backend). */
export function validateName(raw: string): FieldError {
  const name = raw.trim();
  if (!name) return t.validation.nameEmpty;
  if (CONTROL_CHARS.test(name)) return t.validation.nameControl;
  if ([...name].length > MAX_NAME_LEN) return t.validation.nameTooLong(MAX_NAME_LEN);
  return null;
}

/** URL d'un catalogue : http(s), hôte, sans identifiants, 2048 caractères au plus. */
export function validateCatalogUrl(raw: string): FieldError {
  const value = raw.trim();
  if (!value) return t.validation.urlEmpty;
  if (value.length > MAX_URL_LEN) return t.validation.urlTooLong(MAX_URL_LEN);
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return t.validation.urlInvalid;
  }
  if (!["http:", "https:"].includes(url.protocol) || !url.hostname) {
    return t.validation.urlInvalid;
  }
  if (url.username || url.password) return t.validation.urlCredentials;
  return null;
}

/** Chemin facultatif (vide = automatique) : absolu, local, et `.exe` si demandé. */
export function validateOptionalPath(raw: string, options: { exe: boolean }): FieldError {
  const value = raw.trim();
  if (!value) return null;
  if (value.startsWith("\\\\") || value.startsWith("//")) return t.validation.pathNetwork;
  if (!/^[A-Za-z]:[\\/]/.test(value)) return t.validation.pathNotAbsolute;
  if (options.exe && !value.toLowerCase().endsWith(".exe")) {
    return t.validation.pathNotExe;
  }
  return null;
}

/** Taille lisible en français (octets, Ko, Mo, Go ; base 1024). */
export function formatBytes(bytes: number): string {
  const units = ["o", "Ko", "Mo", "Go", "To"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const text =
    unit === 0
      ? String(value)
      : value.toLocaleString("fr-FR", { maximumFractionDigits: 1 });
  return `${text} ${units[unit]}`;
}
