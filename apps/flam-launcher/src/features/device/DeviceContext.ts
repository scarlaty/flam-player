import { createContext, useContext } from "react";
import type {
  CarrierSend,
  DeviceStatus,
  DeviceStories,
  DeviceSummary,
  ReorderReport,
  TransferPhase,
} from "../../api/device";
import type { AppError } from "../../api/error";

export type QueueState = "waiting" | "sending" | "done" | "failed" | "cancelled";

/** Histoire de la file d'envoi. */
export interface QueueItem {
  uuid: string;
  title: string;
  /** Taille du `.plain.pk` en octets (`null` : inconnue). */
  size: number | null;
  state: QueueState;
  /** Message d'erreur (état `failed`). */
  error?: string;
  /** Flam 1.x : clés carrier choisies pour cet envoi. */
  carrier?: CarrierSend;
}

export interface SendRequest {
  uuid: string;
  title: string;
  size: number | null;
}

export type StoriesState =
  | { phase: "loading" }
  | { phase: "error"; error: AppError }
  | { phase: "ready"; report: DeviceStories };

export interface TransferProgressView {
  uuid: string;
  phase: TransferPhase;
  /** Avancement de l'histoire en cours, de 0 à 1. */
  fraction: number;
}

/** Message de fin d'éjection (succès ou repli). */
export interface EjectNote {
  tone: "success" | "warning" | "danger";
  message: string;
}

export interface DeviceApi {
  status: DeviceStatus | null;
  error: AppError | null;
  retry: () => void;
  /** Flam détectée, ou `null`. */
  device: DeviceSummary | null;
  /** La reprise des transferts interrompus de cette détection est terminée. */
  recovered: boolean;
  /** Message de la reprise (reprise effectuée ou échec), s'il y en a un. */
  recoveryNote: EjectNote | null;
  stories: StoriesState;
  /** Relit les histoires de la Flam ; `quiet` garde la liste affichée pendant la lecture. */
  reloadStories: (options?: { quiet?: boolean }) => void;
  firmwareBlocked: boolean;
  queue: QueueItem[];
  progress: TransferProgressView | null;
  /** Annulation demandée, en attente de l'arrêt du transfert en cours. */
  cancelling: boolean;
  /** Un envoi est en attente ou en cours : retrait et éjection sont refusés. */
  busy: boolean;
  /** Demande l'envoi ; l'avertissement « expérimentale » est affiché au premier envoi. */
  requestSend: (request: SendRequest) => void;
  /** L'avertissement « expérimentale » ou le choix des clés carrier est affiché. */
  confirmingSend: boolean;
  /** Flam 1.x : oublie les clés carrier conservées pour la Flam branchée (nombre supprimé). */
  forgetCarrier: () => Promise<number>;
  /** Annule l'envoi en cours et vide la file d'attente. */
  cancelQueue: () => void;
  removeFromQueue: (uuid: string) => void;
  /** Ferme la liste terminée de la file. */
  dismissQueue: () => void;
  /** Identité de la Flam détectée (volume et libellé), `null` sans Flam. */
  deviceKey: string | null;
  removeFromDevice: (
    uuid: string,
    expectedKey: string | null,
  ) => Promise<{ cleanupPending: boolean }>;
  /**
   * Enregistre l'ordre des histoires visibles (tous les UUID de l'index) sur la Flam identifiée
   * par `expectedKey`, puis relit la liste.
   */
  reorderOnDevice: (
    order: string[],
    expectedKey: string | null,
  ) => Promise<ReorderReport>;
  eject: () => Promise<void>;
  ejecting: boolean;
  ejectNote: EjectNote | null;
  /** Texte à annoncer (état de la file, pas de pourcentage). */
  announcement: string;
}

export const DeviceContext = createContext<DeviceApi | null>(null);

/** Accès à la Flam et à la file d'envoi ; nécessite un `DeviceProvider` parent. */
export function useDevice(): DeviceApi {
  const api = useContext(DeviceContext);
  if (!api) throw new Error("useDevice doit être utilisé sous un DeviceProvider");
  return api;
}
