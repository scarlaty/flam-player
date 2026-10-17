import { listen, type UnlistenFn } from "@tauri-apps/api/event";
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

/** Étape d'un transfert (miroir de `Phase` dans `device/transfer.rs`). */
export type TransferPhase = "copy" | "verify";

/** Charge de `device://transfer-progress` : `done` croît de 0 à `total` (copie, puis relecture). */
export interface TransferProgress {
  uuid: string;
  phase: TransferPhase;
  done: number;
  total: number;
}

/** Résultat d'un transfert réussi (miroir de `TransferReport`). */
export interface TransferReport {
  id: string;
  bytes: number;
  files: number;
  replaced: boolean;
  index_updated: boolean;
  cache_removed: boolean;
}

/** Bilan d'une reprise au branchement (miroir de `TransferRecovery`). */
export interface TransferRecovery {
  incomplete_removed: number;
  previous_restored: number;
  updates_finished: number;
  left_alone: number;
}

/** Résultat d'un retrait (miroir de `RemoveReport`). */
export interface RemoveReport {
  id: string;
  index_updated: boolean;
  folders_removed: number;
  cleanup_pending: boolean;
}

/** Résultat d'une éjection (miroir de `EjectOutcome`) : succès ou repli, message prêt à afficher. */
export type EjectOutcome =
  | { status: "ejected"; message: string }
  | { status: "fallback"; reason: string; message: string };

export const TRANSFER_PROGRESS_EVENT = "device://transfer-progress";

/**
 * `device_send_story` : envoie l'histoire `uuid` de la bibliothèque sur la Flam de racine
 * `root` (`validation` si déjà présente, espace insuffisant, firmware non pris en charge ou
 * Flam occupée ; `cancelled` si annulé).
 */
export function sendStoryToDevice(root: string, uuid: string): Promise<TransferReport> {
  return invokeCommand<TransferReport>("device_send_story", { root, uuid });
}

/** `device_cancel_transfer` : demande l'annulation du transfert en cours (faux : aucun). */
export function cancelDeviceTransfer(): Promise<boolean> {
  return invokeCommand<boolean>("device_cancel_transfer");
}

/** `device_recover_transfers` : reprise des transferts et index interrompus. */
export function recoverDeviceTransfers(root: string): Promise<TransferRecovery> {
  return invokeCommand<TransferRecovery>("device_recover_transfers", { root });
}

/** `device_remove_story` : retire l'histoire `id` de la Flam ; `confirmed` doit être vrai. */
export function removeDeviceStory(
  root: string,
  id: string,
  confirmed: boolean,
): Promise<RemoveReport> {
  return invokeCommand<RemoveReport>("device_remove_story", { root, id, confirmed });
}

/** `device_eject` : éjecte la Flam ; l'échec de l'éjection logicielle est un `fallback`. */
export function ejectDevice(root: string): Promise<EjectOutcome> {
  return invokeCommand<EjectOutcome>("device_eject", { root });
}

/** Écoute `device://transfer-progress`. Renvoie la fonction de désabonnement. */
export function onTransferProgress(
  handler: (event: TransferProgress) => void,
): Promise<UnlistenFn> {
  return listen<TransferProgress>(TRANSFER_PROGRESS_EVENT, (e) => handler(e.payload));
}
