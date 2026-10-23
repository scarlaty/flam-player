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
  | { kind: "too_large"; size: number; max: number };

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
  /** Version du firmware lue dans `.mdf` (`1.15.14`, `2.4.2`…), `null` si illisible. */
  firmware: string | null;
  /** État du firmware et de la garde des écritures (#317). Aucune clé ni numéro de série. */
  firmware_status: FirmwareStatus;
}

/** Famille du firmware (miroir de `Family` dans `device/guard.rs`). */
export type FirmwareFamily = "v1" | "v2" | "unsupported" | "unreadable";

/** Bilan du contrôle de cohérence des clés 2.x (miroir de `CoherenceCounts`). */
export interface CoherenceCounts {
  examined: number;
  key_matches: number;
  decrypted: number;
  decrypted_other_key: number;
  lunii_format: number;
  clear_info: number;
}

/** Contrôle de cohérence des clés 2.x (miroir de `Coherence`, informatif). */
export type Coherence =
  | { kind: "not_applicable" }
  | { kind: "not_checked" }
  | { kind: "coherent"; counts: CoherenceCounts }
  | {
      kind: "contradiction";
      counts: CoherenceCounts;
      /** UUID des histoires en cause (au plus 20, triés). */
      stories: string[];
    }
  | { kind: "no_evidence"; counts: CoherenceCounts };

/** Confiance accordée à une Flam 2.x (miroir de `Trust`). */
export type FirmwareTrust =
  "not_applicable" | "flam_verified" | "confirmed" | "unconfirmed";

/** Refus d'écrire de la garde (miroir de `WriteRefused`). */
export type WriteRefused =
  | { kind: "device_not_found" }
  | { kind: "invalid_root" }
  | { kind: "mdf_unreadable"; detail: string }
  | { kind: "truncated"; size: number }
  | { kind: "version_unreadable" }
  | { kind: "unsupported_firmware"; version: string }
  | { kind: "keys_unreadable"; detail: string }
  | { kind: "keys_trivial"; version: string }
  | { kind: "keys_contradiction"; version: string }
  | { kind: "confirmation_required"; version: string; standard: boolean };

/** État du firmware (miroir de `FirmwareStatus` dans `device/guard.rs`). */
export interface FirmwareStatus {
  version: string | null;
  family: FirmwareFamily;
  version_form: "main" | "bare" | "unreadable";
  mdf_size: number;
  /** Deux premiers octets de `.mdf` (`01 00`). */
  header: string | null;
  /** 126 octets et en-tête `01 00` (seule disposition mesurée). */
  layout_standard: boolean;
  /** VID:PID USB lus dans `.mdf` (`303a:819e`). */
  usb_ids: string | null;
  coherence: Coherence;
  trust: FirmwareTrust;
  /** Raison du refus d'écrire ; `null` : écriture autorisée. */
  write_block: WriteRefused | null;
}

/** Le firmware exige le mode carrier (1.x). */
export function needsCarrier(device: DeviceSummary | null): boolean {
  if (!device) return false;
  if (device.firmware_status) return device.firmware_status.family === "v1";
  return device.firmware?.split(".")[0] === "1";
}

/**
 * Refus d'envoyer sur la Flam d'après l'état du firmware (`null` : envoi possible, éventuellement
 * après confirmation, voir `needsFirmwareConfirmation`).
 */
export function firmwareWriteBlock(device: DeviceSummary | null): WriteRefused | null {
  const block = device?.firmware_status?.write_block ?? null;
  return block && block.kind !== "confirmation_required" ? block : null;
}

/**
 * Flam 2.x pas encore confirmée : l'utilisateur doit confirmer avant toute écriture. Vrai aussi
 * quand l'envoi est bloqué par une contradiction des clés : la confirmation rend alors le
 * retrait de l'histoire en cause possible.
 */
export function needsFirmwareConfirmation(device: DeviceSummary | null): boolean {
  const status = device?.firmware_status;
  const kind = status?.write_block?.kind;
  return (
    kind === "confirmation_required" ||
    (kind === "keys_contradiction" && status?.trust === "unconfirmed")
  );
}

/**
 * Reprise, retrait et ordre possibles : seule une contradiction des clés (qui ne refuse que
 * l'envoi) est tolérée.
 */
export function canMaintain(device: DeviceSummary | null): boolean {
  if (!device) return false;
  const status = device.firmware_status;
  if (!status) return true;
  if (status.trust === "unconfirmed") return false;
  const block = status.write_block;
  return block === null || block.kind === "keys_contradiction";
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
  /**
   * Titre de la bibliothèque du launcher (associé par `uuid.bin`). Le titre déchiffré de la Flam
   * arrive ensuite par `getDeviceStoryTitles` et prime sur celui-ci.
   */
  title: string | null;
  /** L'histoire figure dans la bibliothèque du launcher. */
  in_library: boolean;
  /** Listée dans l'index mais sans dossier sur la Flam. */
  missing: boolean;
  /**
   * Identifiants de bibliothèque des histoires installées sous cet UUID (associées par
   * `uuid.bin`) : une source dossier ou Telmi n'a pas l'UUID de la Flam pour identifiant.
   */
  library_uuids: string[];
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
 * `device_diagnostic` : diagnostic Markdown à coller dans une issue « Retour testeur Flam »
 * (#318). Lecture seule, sans réseau ; disponible sans Flam (diagnostic réduit). Ne contient ni
 * clé, ni `bt`, ni numéro de série, ni titre ou UUID d'histoire, ni nom d'utilisateur.
 */
export function getDeviceDiagnostic(): Promise<string> {
  return invokeCommand<string>("device_diagnostic");
}

/**
 * `device_list_stories` : histoires de la Flam (`null` : aucune Flam détectée). Lecture seule,
 * sans reprise d'index.
 */
export function listDeviceStories(): Promise<DeviceStories | null> {
  return invokeCommand<DeviceStories | null>("device_list_stories");
}

/** Titre déchiffré d'une histoire de la Flam (miroir de `StoryTitle` dans `device/details.rs`). */
export interface DeviceStoryTitle {
  uuid: string;
  /** `null` : pas de clé pour cette Flam, ou `info` illisible (repli de l'interface). */
  title: string | null;
}

/** Réponse de `device_story_titles` (miroir de `StoryTitles`). */
export interface DeviceStoryTitles {
  /** Histoires traitées, dans l'ordre de la demande (UUID invalides omis). */
  titles: DeviceStoryTitle[];
  /** Lot interrompu (opération en cours sur la Flam, annulation) : redemander le reste plus tard. */
  interrupted: boolean;
  /** Des clés de titre étaient disponibles ; faux : les `null` ne sont pas définitifs. */
  keys: boolean;
}

/** Réponse de `device_story_thumbnail` (miroir de `StoryThumbnail`). */
export interface DeviceStoryThumbnail {
  /** URL `data:image/png` ; `null` : vignette absente ou invalide. */
  thumb: string | null;
  /** Opération en cours sur la Flam : rien n'a été lu, redemander plus tard. */
  busy: boolean;
}

/** Nombre maximal d'histoires par appel de `getDeviceStoryTitles` (`MAX_BATCH`). */
export const STORY_TITLES_BATCH = 100;

/**
 * `device_story_titles` : titres déchiffrés d'un lot d'histoires de la Flam de `root` (lecture
 * seule ; Flam 1.x : seulement avec la clé carrier déjà conservée et le consentement à jour).
 */
export function getDeviceStoryTitles(
  root: string,
  uuids: string[],
): Promise<DeviceStoryTitles> {
  return invokeCommand<DeviceStoryTitles>("device_story_titles", { root, uuids });
}

/** `device_story_thumbnail` : vignette d'une histoire de la Flam (lecture seule, mise en cache). */
export function getDeviceStoryThumbnail(
  root: string,
  uuid: string,
): Promise<DeviceStoryThumbnail> {
  return invokeCommand<DeviceStoryThumbnail>("device_story_thumbnail", { root, uuid });
}

/** Histoire trouvée dans le catalogue officiel (miroir de `OfficialTitle` dans `device/official.rs`). */
export interface OfficialStoryTitle {
  uuid: string;
  title: string;
  /** Le catalogue a une image pour cette histoire (`getOfficialStoryThumbnail`). */
  has_image: boolean;
}

/** Réponse de `device_official_titles` (miroir de `OfficialTitles`). */
export interface OfficialStoryTitles {
  /** Histoires demandées que le catalogue connaît (les autres sont omises). */
  titles: OfficialStoryTitle[];
}

/**
 * `device_official_titles` : titre et image du catalogue officiel en ligne pour des histoires
 * dont le titre n'a pas pu être déchiffré. Le launcher n'appelle l'API qu'à cette condition (au
 * plus une fois par jour) ; aucune donnée de la Flam n'est envoyée au serveur.
 */
export function getOfficialStoryTitles(uuids: string[]): Promise<OfficialStoryTitles> {
  return invokeCommand<OfficialStoryTitles>("device_official_titles", { uuids });
}

/** `device_official_thumbnail` : image du catalogue pour une histoire trouvée (`null` : indisponible). */
export function getOfficialStoryThumbnail(uuid: string): Promise<string | null> {
  return invokeCommand<string | null>("device_official_thumbnail", { uuid });
}

/** `device_story_details_cancel` : interrompt le lot de titres en cours. */
export function cancelDeviceStoryDetails(): Promise<void> {
  return invokeCommand<void>("device_story_details_cancel");
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
  /** Mode carrier : `info` relu sur la Flam se déchiffre avec le couple (`null` hors carrier). */
  carrier_readback: boolean | null;
  /** Mode carrier : les clés sont conservées sur ce PC pour cette Flam (`null` hors carrier). */
  carrier_stored: boolean | null;
  /**
   * Reprises de transferts interrompus qui ont échoué avant l'envoi (« entrée : erreur ») :
   * l'envoi a réussi, mais une autre histoire peut rester incomplète. Vide d'ordinaire.
   */
  recovery_failures: string[];
}

/** Consentement à l'avertissement du mode carrier (miroir de `CarrierConsent`). */
export interface CarrierConsent {
  text_version: number;
  /** Date UTC ISO 8601. */
  accepted_at: string;
}

/** Histoire officielle utilisable comme carrier (miroir de `CarrierCandidate`, sans clé). */
export interface CarrierCandidate {
  uuid: string;
  title: string;
  hidden: boolean;
  /** Histoire sans `bt` : son titre se déchiffre avec la clé conservée pour cette Flam. */
  with_stored: boolean;
}

/** État du mode carrier pour une Flam (miroir de `CarrierState`, sans clé). */
export interface CarrierState {
  warning_text: string;
  warning_version: number;
  consent: CarrierConsent | null;
  firmware: string | null;
  carrier_needed: boolean;
  stored_keys: boolean;
  candidates: CarrierCandidate[];
}

/** Source du couple (miroir de `CarrierChoice`). */
export type CarrierChoice =
  | { kind: "stored" }
  | { kind: "device"; uuid: string }
  | { kind: "file"; path: string }
  /**
   * `bt` saisi : 64 caractères hexadécimaux (revérifié par le backend). `confirmed` : titre de
   * l'histoire reconnue confirmé par l'utilisateur (sinon refus du backend).
   */
  | { kind: "entered"; bt: string; confirmed: boolean };

/** Histoire reconnue par un `bt` saisi (miroir de `EnteredPreview`, sans clé). */
export interface EnteredPreview {
  uuid: string;
  title: string;
  hidden: boolean;
}

/**
 * `device_carrier_preview` : histoire de la Flam reconnue par le `bt` saisi, en lecture seule,
 * pour que l'utilisateur vérifie son titre avant d'envoyer.
 */
export function previewEnteredBt(root: string, bt: string): Promise<EnteredPreview> {
  return invokeCommand<EnteredPreview>("device_carrier_preview", { root, bt });
}

/** Nombre de chiffres hexadécimaux d'un `bt` (32 octets). */
export const BT_HEX_DIGITS = 64;

/**
 * `bt` saisi, sans espaces, s'il compte exactement 64 chiffres hexadécimaux (majuscules
 * tolérées) ; sinon `null`.
 */
export function normalizeBtHex(text: string): string | null {
  const compact = text.replace(/\s+/g, "");
  return compact.length === BT_HEX_DIGITS && /^[0-9a-fA-F]+$/.test(compact)
    ? compact.toLowerCase()
    : null;
}

/** Demande carrier jointe à un envoi : source et conservation sur ce PC. */
export interface CarrierSend {
  choice: CarrierChoice;
  remember: boolean;
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

/** Résultat d'un réordonnancement (miroir de `ReorderReport` dans `device/reorder.rs`). */
export interface ReorderReport {
  /** L'index a été réécrit (faux : l'ordre était déjà celui de la Flam). */
  changed: boolean;
  /** `library.cache` supprimé : la Flam reconstruit sa bibliothèque au redémarrage. */
  cache_removed: boolean;
  count: number;
}

/** Résultat d'une éjection (miroir de `EjectOutcome`) : succès ou repli, message prêt à afficher. */
export type EjectOutcome =
  | { status: "ejected"; message: string }
  | { status: "fallback"; reason: string; message: string };

export const TRANSFER_PROGRESS_EVENT = "device://transfer-progress";

/**
 * `device_send_story` : envoie l'histoire `uuid` de la bibliothèque sur la Flam de racine
 * `root` (`validation` si déjà présente sans `replace`, espace insuffisant, firmware non pris
 * en charge ou Flam occupée ; `cancelled` si annulé). `replace` : UUID de l'histoire de la Flam
 * à remplacer (même à version égale), l'ancienne n'étant retirée qu'une fois la nouvelle en
 * place ; refusé (`validation`) s'il ne désigne pas l'histoire de l'archive.
 */
export function sendStoryToDevice(
  root: string,
  uuid: string,
  carrier?: CarrierSend,
  replace?: string,
): Promise<TransferReport> {
  const args: Record<string, unknown> = carrier
    ? { root, uuid, carrier: carrier.choice, remember: carrier.remember }
    : { root, uuid };
  if (replace) args.replace = replace;
  return invokeCommand<TransferReport>("device_send_story", args);
}

/**
 * `device_carrier_state` : avertissement, consentement, firmware, clés conservées et histoires
 * officielles utilisables (listées seulement après consentement). Aucune clé.
 */
export function getCarrierState(root: string): Promise<CarrierState> {
  return invokeCommand<CarrierState>("device_carrier_state", { root });
}

/** `device_carrier_accept` : enregistre le consentement (case « J'ai compris » cochée). */
export function acceptCarrierWarning(
  textVersion: number,
  understood: boolean,
): Promise<CarrierConsent> {
  return invokeCommand<CarrierConsent>("device_carrier_accept", {
    textVersion,
    understood,
  });
}

/** `device_carrier_forget` : oublie les clés conservées pour la Flam de `root`. */
export function forgetCarrierKeys(root: string): Promise<number> {
  return invokeCommand<number>("device_carrier_forget", { root });
}

/**
 * `device_confirm_firmware` : « Firmware 2.x en test : j'ai sauvegardé ma Flam » (`understood` :
 * case cochée), mémorisé sur ce PC pour cette Flam. Rend l'état du firmware à jour.
 */
export function confirmFirmware(
  root: string,
  understood: boolean,
): Promise<FirmwareStatus> {
  return invokeCommand<FirmwareStatus>("device_confirm_firmware", {
    root,
    understood,
  });
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

/**
 * `device_reorder_stories` : réécrit l'index de la Flam de `root` dans l'ordre `order` (tous les
 * UUID de l'index, chacun une fois). `validation` si l'ordre ne correspond plus à l'index, si
 * l'index est abîmé ou si la Flam est occupée ; rien n'est alors écrit.
 */
export function reorderDeviceStories(
  root: string,
  order: string[],
): Promise<ReorderReport> {
  return invokeCommand<ReorderReport>("device_reorder_stories", { root, order });
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
