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
import type { LibraryFormat } from "../../api/library";

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
  /** UUID de l'histoire de la Flam à remplacer. */
  replace?: string;
  /** Envoi réussi, avec une remarque (reprise d'un autre transfert en échec). */
  warning?: string;
  /** Relecture de la Flam qui suit l'envoi réussi (numéro interne). */
  reloadGen?: number;
  /** Envoi réussi dont la relecture de la Flam n'est pas encore revenue : bloque un renvoi. */
  awaitingList?: boolean;
  /**
   * Envoi réussi : copie de l'histoire dans « Mes histoires » (#609). `kept` : « Retirer du PC »
   * proposé ; `removing` : retrait en cours ; `removed` : retirée (l'histoire reste sur la Flam) ;
   * `failed` : retrait impossible (`pcCopyError`), « Retirer du PC » reproposé. Absent : rien
   * n'est proposé.
   */
  pcCopy?: PcCopyState;
  /** Message du retrait impossible (`failed`) ou refusé (`refused`). */
  pcCopyError?: string;
  /** Identifiant de l'histoire sur la Flam (rapport d'envoi), pour suivre son retrait. */
  deviceId?: string;
  /** Retrait automatique demandé mais non fait : écriture sur la Flam non confirmée. */
  pcCopyUnconfirmed?: boolean;
  /**
   * Envoi demandé par « Installer et envoyer » (#609), parti ou non. Ces envois arrivent seuls,
   * au fil des installations : ils ne remplacent pas une file terminée, et leurs lignes en échec
   * (envoi non parti : Flam changée, déjà présente, espace insuffisant…) restent affichées
   * jusqu'à la fermeture de la liste.
   */
  planned?: boolean;
}

/**
 * Envoi prévu à la fin d'une installation (« Installer et envoyer », #609). Il ne part seul que
 * sur la Flam branchée au moment du clic (`deviceKey`), si elle n'a jamais cessé d'être vue
 * entre-temps et si la scrutation n'a jamais été suspendue (`unwatched`).
 */
export interface PlannedSend {
  uuid: string;
  title: string;
  format: LibraryFormat;
  /** Identité de la Flam au moment du clic (volume et libellé). */
  deviceKey: string;
  /**
   * `pending` : demandé, installation pas encore démarrée (confirmation en attente : rien n'est
   * affiché) ; `installing` : installation en cours ; `ready` : installée, envoi sur le point de
   * partir, ou à confirmer (`unwatched`).
   */
  state: "pending" | "installing" | "ready";
  /** Taille du `.plain.pk` installé (`ready`), `null` : inconnue. */
  size: number | null;
  /** La Flam du clic a été débranchée ou remplacée depuis : l'envoi ne partira pas. */
  lost: boolean;
  /**
   * La scrutation de la Flam a été suspendue depuis le clic (fenêtre masquée, lecture
   * incrustée) : rien ne prouve que la Flam branchée est celle du clic. L'envoi ne part pas
   * seul : une fois l'histoire installée, il est à confirmer par « Envoyer sur la Flam ».
   */
  unwatched: boolean;
  /** L'histoire installée a été vue dans « Mes histoires » (`ready`) : sa disparition l'oublie. */
  inLibrary?: boolean;
}

export interface PlannedSendRequest {
  uuid: string;
  title: string;
  format: LibraryFormat;
}

/**
 * `refused` : « Retirer du PC » refusé par le backend (histoire plus sur la Flam, autre Flam…) :
 * le bouton n'est plus proposé.
 */
export type PcCopyState = "kept" | "removing" | "removed" | "failed" | "refused";

/** Ce dont l'application est prévenue pour la copie « Mes histoires » d'un envoi (#609). */
export type PcCopyNotice =
  | { status: "removed" }
  | { status: "remove_failed"; message: string }
  | { status: "kept_unconfirmed" };

export interface SendRequest {
  uuid: string;
  title: string;
  size: number | null;
  /**
   * UUID sur la Flam de l'histoire à remplacer (« Remplacer sur la Flam ») : le backend refuse
   * si l'archive envoyée en désigne une autre.
   */
  replace?: string;
  /** Envoi demandé par « Installer et envoyer » (voir `QueueItem.planned`). */
  planned?: boolean;
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
  /** Relance la scrutation ; renvoie le numéro de la demande (voir `retryAnswered`). */
  retry: () => number;
  /** Numéro de la dernière demande de `retry` à laquelle une réponse est arrivée. */
  retryAnswered: number;
  /** Flam détectée, ou `null`. */
  device: DeviceSummary | null;
  /** La reprise des transferts interrompus de cette détection est terminée. */
  recovered: boolean;
  /** Message de la reprise (reprise effectuée ou échec), s'il y en a un. */
  recoveryNote: EjectNote | null;
  stories: StoriesState;
  /** Relit les histoires de la Flam ; `quiet` garde la liste affichée pendant la lecture. */
  reloadStories: (options?: { quiet?: boolean }) => void;
  /**
   * Envoi refusé sur cette Flam d'après l'état structuré du firmware (`device_status`, #317) :
   * firmware non pris en charge, `.mdf` illisible ou tronqué, clés invalides.
   */
  firmwareBlocked: boolean;
  /**
   * Reprise, retrait et ordre autorisés par la garde du firmware : faux sans Flam ou si
   * l'écriture est bloquée (une contradiction des clés n'est qu'un avertissement, #326).
   */
  canMaintain: boolean;
  queue: QueueItem[];
  progress: TransferProgressView | null;
  /** Annulation demandée, en attente de l'arrêt du transfert en cours. */
  cancelling: boolean;
  /** Un envoi est en attente ou en cours : retrait et éjection sont refusés. */
  busy: boolean;
  /**
   * Demande l'envoi ; l'avertissement « expérimentale » est affiché au premier envoi. Une
   * demande faite par l'utilisateur oublie l'envoi prévu de la même histoire.
   */
  requestSend: (request: SendRequest) => void;
  /** L'avertissement « expérimentale » ou le choix des clés carrier est affiché. */
  confirmingSend: boolean;
  /** Flam 1.x : oublie les clés carrier conservées pour la Flam branchée (nombre supprimé). */
  forgetCarrier: () => Promise<number>;
  /**
   * Envois prévus à la fin d'une installation (« Installer et envoyer », #609), dans l'ordre des
   * demandes : ils partent dans cet ordre, par la file d'envoi.
   */
  plannedSends: PlannedSend[];
  /**
   * « Installer et envoyer » : prévoit l'envoi de l'histoire sur la Flam branchée, puis appelle
   * `start` (qui lance l'installation). L'avertissement « expérimentale » est demandé d'abord :
   * s'il est refusé, rien n'est prévu ni lancé. Sans Flam, ne fait rien. Le démarrage réel de
   * l'installation est signalé par `startPlannedSend`, sa fin par `fulfilPlannedSend` ou
   * `dropPlannedSend`.
   */
  planSend: (request: PlannedSendRequest, start: () => void) => void;
  /** L'installation a réellement démarré (confirmations passées) : l'envoi prévu est affiché. */
  startPlannedSend: (uuid: string) => void;
  /**
   * L'installation de l'histoire a réussi (`size` : taille installée) : son envoi prévu part par
   * la file d'envoi, à son tour, si la Flam est restée celle du clic et si les règles d'envoi
   * l'acceptent ; sinon une ligne en échec dit pourquoi. Si la scrutation a été suspendue depuis
   * le clic, il attend d'être confirmé. Sans envoi prévu, ne fait rien.
   */
  fulfilPlannedSend: (uuid: string, size: number | null) => void;
  /** Installation en échec, annulée ou abandonnée, ou envoi prévu retiré : rien ne partira. */
  dropPlannedSend: (uuid: string) => void;
  /** Annule l'envoi en cours, vide la file d'attente et oublie les envois prévus. */
  cancelQueue: () => void;
  removeFromQueue: (uuid: string) => void;
  /** Ferme la liste terminée de la file. */
  dismissQueue: () => void;
  /**
   * « Retirer du PC » (#609) : retire de « Mes histoires » une histoire envoyée avec succès
   * (`pcCopy` `kept` ou `failed`) ; elle reste sur la Flam. Le résultat est porté par `pcCopy`.
   */
  removeFromPc: (uuid: string) => Promise<void>;
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
  /** Un retrait de la Flam est en cours (l'éjection est alors refusée). */
  removing: boolean;
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
