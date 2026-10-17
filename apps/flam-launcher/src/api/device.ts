import { invokeCommand } from "./error";

/** Espace disque d'un volume, en octets (miroir de `Space` dans `device/detect.rs`). */
export interface DeviceSpace {
  free: number;
  total: number;
}

/** Avertissement sur la Flam détectée (miroir de `DeviceWarning`). */
export type DeviceWarning =
  | { kind: "missing_str_dir" }
  | { kind: "missing_library_dir" }
  | { kind: "unexpected_mdf_size"; size: number };

/** Problème de lecture de `.mdf` (miroir de `MdfError`, sans « format non spécifié »). */
export type MdfProblem =
  | { kind: "truncated"; size: number; expected_min: number }
  | { kind: "too_large"; size: number; max: number }
  | { kind: "format_unspecified" };

/** Flam détectée (miroir de `DeviceSummary` dans `device/status.rs`). */
export interface DeviceSummary {
  /** Lettre ou chemin du volume (`E:\`). */
  root: string;
  volume_label: string;
  file_system: string;
  space: DeviceSpace | null;
  warnings: DeviceWarning[];
  mdf_problem: MdfProblem | null;
  /** La sauvegarde personnelle de `.mdf` a échoué. */
  backup_failed: boolean;
}

/** Réponse de `device_status` (miroir de `DeviceStatus`). */
export interface DeviceStatus {
  detected: boolean;
  hint: string | null;
  device: DeviceSummary | null;
  /** Nombre de Flam détectées (seule la première est décrite). */
  device_count: number;
}

export type IndexHealth = "valid" | "absent" | "damaged" | "unreadable";

export type Placement = "visible" | "hidden" | "unlisted";

/** Histoire de la Flam (miroir de `DeviceStory`). */
export interface DeviceStory {
  uuid: string;
  /** Huit premiers caractères de l'UUID. */
  short_id: string;
  placement: Placement;
  /** Titre connu de la bibliothèque du launcher (celui de la Flam est chiffré). */
  title: string | null;
  /** L'histoire figure dans la bibliothèque du launcher : vignette disponible. */
  in_library: boolean;
  /** Listée dans l'index mais sans dossier sur la Flam. */
  missing: boolean;
}

/** Réponse de `device_list_stories` (miroir de `StoriesReport`). */
export interface DeviceStories {
  stories: DeviceStory[];
  visible_index: IndexHealth;
  hidden_index: IndexHealth;
  /** Liste coupée à `limit` histoires. */
  truncated: boolean;
  limit: number;
  /** `str/` illisible : seuls les index sont affichés. */
  str_unreadable: boolean;
}

/**
 * `device_status` : détecte la Flam branchée (lecture seule ; seule écriture : la sauvegarde
 * personnelle de `.mdf` dans le dossier de données du launcher).
 */
export function getDeviceStatus(): Promise<DeviceStatus> {
  return invokeCommand<DeviceStatus>("device_status");
}

/**
 * `device_list_stories` : histoires de la Flam (`null` : aucune Flam détectée). Lecture seule,
 * sans reprise d'index.
 */
export function listDeviceStories(): Promise<DeviceStories | null> {
  return invokeCommand<DeviceStories | null>("device_list_stories");
}
