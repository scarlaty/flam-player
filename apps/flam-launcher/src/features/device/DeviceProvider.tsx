import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  cancelDeviceTransfer,
  ejectDevice,
  canMaintain,
  DEVICE_CHANGED,
  firmwareWriteBlock,
  forgetCarrierKeys,
  listDeviceStories,
  LUNII_NEEDS_V2,
  needsCarrier,
  onTransferProgress,
  recoverDeviceTransfers,
  removeDeviceStory,
  removeSentFromLibrary,
  reorderDeviceStories,
  sendStoryToDevice,
  type CarrierSend,
  type DeviceSummary,
  type LibraryCopy,
} from "../../api/device";
import { toAppError } from "../../api/error";
import { Banner, Button, Dialog } from "../../components";
import { t } from "../../i18n";
import { getFlamSendWarningAck, setFlamSendWarningAck } from "../settings/preferences";
import {
  DeviceContext,
  type DeviceApi,
  type EjectNote,
  type PcCopyNotice,
  type PcCopyState,
  type PlannedSend,
  type PlannedSendRequest,
  type QueueItem,
  type SendRequest,
  type StoriesState,
  type TransferProgressView,
} from "./DeviceContext";
import { CarrierDialog } from "./CarrierDialog";
import { sendErrorMessage } from "./sendError";
import { REASON_TEXT } from "./sendReasons";
import { sendBlock } from "./sendRules";
import { useDeviceStatus } from "./useDeviceStatus";

const text = t.deviceTransfer;

/** Intervalle des tentatives d'annulation tant que le transfert n'a pas pris le drapeau. */
const CANCEL_RETRY_MS = 200;
const CANCEL_RETRIES = 15;

export interface DeviceProviderProps {
  children: ReactNode;
  /** Pas de scrutation (player incrusté). */
  paused?: boolean;
  /** Délai entre deux scrutations (défaut : celui de `useDeviceStatus`). */
  intervalMs?: number;
  /**
   * Identifiants des histoires de la bibliothèque. Quand il est fourni, les envois réussis d'une
   * histoire qui n'y figure plus (supprimée) sont retirés de la file affichée.
   */
  libraryUuids?: ReadonlySet<string>;
  /**
   * Une histoire envoyée a été retirée de « Mes histoires » (réglage ou « Retirer du PC »), ou
   * son retrait a échoué (#609) : la bibliothèque est à relire, l'utilisateur à prévenir.
   */
  onPcCopyChange?: (title: string, notice: PcCopyNotice) => void;
}

/** `kind` des refus définitifs de `library_remove_sent` (les autres sont passagers). */
const DEFINITIVE_REFUSALS: ReadonlySet<string> = new Set(["not_removable", "not_found"]);

/** Même histoire de la Flam (UUID avec ou sans tirets, casse indifférente). */
function sameStory(a: string | undefined, b: string): boolean {
  const norm = (id: string) => id.replace(/-/g, "").toLowerCase();
  return a !== undefined && norm(a) === norm(b);
}

/** « Retirer du PC » encore proposé (ou en cours) pour cette ligne. */
function offersPcRemoval(item: QueueItem): boolean {
  return item.pcCopy === "kept" || item.pcCopy === "failed";
}

/** État affiché de la copie « Mes histoires » d'après le rapport d'envoi (#609). */
function pcCopyOf(copy: LibraryCopy | null | undefined): {
  pcCopy?: PcCopyState;
  pcCopyError?: string;
  pcCopyUnconfirmed?: boolean;
} {
  switch (copy?.status) {
    case "kept":
      return { pcCopy: "kept", pcCopyUnconfirmed: copy.unconfirmed === true };
    case "removed":
      return { pcCopy: "removed" };
    case "remove_failed":
      return { pcCopy: "failed", pcCopyError: copy.message };
    default:
      return {};
  }
}

/**
 * Tient la Flam branchée (détection, reprise au branchement, histoires), la file d'envoi
 * séquentielle et les actions Retirer / Éjecter. Placé au-dessus des écrans : la file survit à
 * un changement d'écran.
 */
export function DeviceProvider({
  children,
  paused = false,
  intervalMs,
  libraryUuids,
  onPcCopyChange,
}: DeviceProviderProps) {
  const { status, error, retry, answered } = useDeviceStatus({
    enabled: !paused,
    intervalMs,
  });
  const device = status?.detected ? status.device : null;
  const root = device?.root ?? null;
  const deviceKey = device ? `${device.root}\u0000${device.volume_label}` : null;
  const known = status !== null;
  // Garde des écritures (#317) : l'état structuré du firmware décide, pas le texte des erreurs.
  const firmwareBlocked = firmwareWriteBlock(device) !== null;
  // Reprise, retrait et ordre possibles : la garde ne refuse rien (une contradiction des clés
  // n'est qu'un avertissement, #326).
  const writable = canMaintain(device);

  const rootRef = useRef<string | null>(null);
  const deviceKeyRef = useRef<string | null>(null);
  const deviceRef = useRef<DeviceSummary | null>(null);
  const onPcCopyChangeRef = useRef(onPcCopyChange);
  useEffect(() => {
    rootRef.current = root;
    deviceKeyRef.current = deviceKey;
    deviceRef.current = device;
    onPcCopyChangeRef.current = onPcCopyChange;
  });

  // Reprise : une fois par branchement (changement de volume ou retour après une absence).
  const startedKey = useRef<string | null>(null);
  // Reprise différée par la garde du firmware : la liste, déjà lue, sera relue après elle.
  const deferredKey = useRef<string | null>(null);
  const [recoveredKey, setRecoveredKey] = useState<string | null>(null);
  const [recoveryNote, setRecoveryNote] = useState<EjectNote | null>(null);
  const [stories, setStories] = useState<StoriesState>({ phase: "loading" });
  const [reload, setReload] = useState(0);
  // Compteur des relectures demandées, et dernière relecture terminée : un envoi réussi reste
  // « en file » tant que la relecture qui suit n'est pas revenue (pas de double envoi).
  const reloadGen = useRef(0);
  const [settledReload, setSettledReload] = useState(0);
  const bumpReload = useCallback(() => {
    reloadGen.current += 1;
    setReload(reloadGen.current);
  }, []);
  const [ejectNote, setEjectNote] = useState<EjectNote | null>(null);

  // Mode carrier (Flam 1.x) : demande en attente du choix des clés (déclarée ici, la remise à
  // zéro ci-dessous la lit).
  const [pendingCarrier, setPendingCarrier] = useState<SendRequest | null>(null);

  // Nouvelle Flam, retour après une absence ou perte : l'état lié à la détection repart de zéro
  // (ajustement pendant le rendu, sans effet).
  const [seenKey, setSeenKey] = useState<string | null>(null);
  if (known && seenKey !== deviceKey) {
    setSeenKey(deviceKey);
    setRecoveredKey(null);
    setRecoveryNote(null);
    setStories({ phase: "loading" });
    // Le dialogue des clés se démonte avec la Flam : une demande laissée en attente bloquerait
    // les fiches d'histoire (`confirmingSend`) et resurgirait au rebranchement (#572).
    setPendingCarrier(null);
    if (deviceKey !== null) setEjectNote(null);
  }

  useEffect(() => {
    if (!known) return;
    if (deviceKey === null || root === null) {
      startedKey.current = null;
      return;
    }
    if (startedKey.current === deviceKey) return;
    const key = deviceKey;
    if (!writable) {
      deferredKey.current = key;
      // La garde du firmware refuse toute écriture, reprise comprise (#317) : la liste se lit
      // quand même ; la reprise est tentée si l'écriture devient possible (confirmation).
      void Promise.resolve().then(() => {
        if (deviceKeyRef.current === key) setRecoveredKey(key);
      });
      return;
    }
    startedKey.current = key;
    if (running.current) {
      // Un envoi tient le verrou de la Flam (simple absence transitoire) : rien à reprendre.
      void Promise.resolve().then(() => {
        if (startedKey.current === key) setRecoveredKey(key);
      });
      return;
    }
    recoverDeviceTransfers(root)
      .then((report) => {
        if (
          startedKey.current === key &&
          report.incomplete_removed + report.previous_restored + report.updates_finished >
            0
        ) {
          setRecoveryNote({ tone: "success", message: text.recovery.done });
        }
      })
      .catch((caught) => {
        if (startedKey.current === key) {
          setRecoveryNote({
            tone: "warning",
            message: text.recovery.failed(toAppError(caught).message),
          });
        }
      })
      .finally(() => {
        if (startedKey.current !== key) return;
        setRecoveredKey(key);
        if (deferredKey.current === key) {
          deferredKey.current = null;
          bumpReload();
        }
      });
  }, [known, deviceKey, root, writable, bumpReload]);

  const recovered = deviceKey !== null && recoveredKey === deviceKey;

  useEffect(() => {
    if (!recovered) return;
    let active = true;
    const gen = reload;
    listDeviceStories()
      .then((report) => {
        if (!active) return;
        setSettledReload(gen);
        setStories(
          report
            ? { phase: "ready", report }
            : {
                phase: "error",
                error: { kind: "not_found", message: t.deviceScreen.stories.noAnswer },
              },
        );
      })
      .catch((caught) => {
        if (!active) return;
        setSettledReload(gen);
        setStories({ phase: "error", error: toAppError(caught) });
      });
    return () => {
      active = false;
    };
  }, [recovered, deviceKey, reload]);

  const reloadStories = useCallback(
    (options?: { quiet?: boolean }) => {
      if (!options?.quiet) setStories({ phase: "loading" });
      bumpReload();
    },
    [bumpReload],
  );

  // --- File d'envoi ---------------------------------------------------------------------
  const queueRef = useRef<QueueItem[]>([]);
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const [progress, setProgress] = useState<TransferProgressView | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const [announcement, setAnnouncement] = useState("");
  const running = useRef(false);
  // Un retrait d'histoire de la Flam est en cours.
  const [removing, setRemoving] = useState(false);
  const sendingUuid = useRef<string | null>(null);
  // Action en attente de l'avertissement « expérimentale » (envoi, ou « Installer et envoyer »).
  const [pendingAck, setPendingAck] = useState<{ run: () => void } | null>(null);
  // Choix retenu pour la Flam branchée pendant cette session (mode carrier).
  const carrierSession = useRef<{ key: string | null; carrier: CarrierSend } | null>(
    null,
  );
  const cancelButton = useRef<HTMLButtonElement>(null);

  const exposedQueue = useMemo(
    () =>
      queue.map((item) =>
        item.state === "done" && (item.reloadGen ?? 0) > settledReload
          ? { ...item, awaitingList: true }
          : item,
      ),
    [queue, settledReload],
  );

  const updateQueue = useCallback((change: (items: QueueItem[]) => QueueItem[]) => {
    queueRef.current = change(queueRef.current);
    setQueue(queueRef.current);
  }, []);

  const setItemState = useCallback(
    (uuid: string, state: QueueItem["state"], error?: string) =>
      updateQueue((items) =>
        items.map((item) => (item.uuid === uuid ? { ...item, state, error } : item)),
      ),
    [updateQueue],
  );

  /** Retire « Retirer du PC » des lignes choisies (histoire retirée de la Flam, autre Flam). */
  const clearPcCopy = useCallback(
    (which: (item: QueueItem) => boolean) =>
      updateQueue((items) =>
        items.some((item) => offersPcRemoval(item) && which(item))
          ? items.map((item) =>
              offersPcRemoval(item) && which(item)
                ? { ...item, pcCopy: undefined, pcCopyError: undefined }
                : item,
            )
          : items,
      ),
    [updateQueue],
  );

  // Flam changée, débranchée ou rebranchée : « Retirer du PC » n'est plus proposé (le backend
  // revérifierait de toute façon que l'histoire est sur la même Flam).
  const pcCopyKey = useRef(deviceKey);
  useEffect(() => {
    if (pcCopyKey.current === deviceKey) return;
    pcCopyKey.current = deviceKey;
    clearPcCopy(() => true);
  }, [deviceKey, clearPcCopy]);

  /**
   * Les histoires en attente passent à « annulé » (elles pourront être renvoyées) ; `only` :
   * seulement celles-ci (en attente au résultat de l'envoi, pas un ajout fait ensuite).
   */
  const cancelWaiting = useCallback(
    (only?: ReadonlySet<string>) =>
      updateQueue((items) =>
        items.map((item) =>
          item.state === "waiting" && (!only || only.has(item.uuid))
            ? { ...item, state: "cancelled" }
            : item,
        ),
      ),
    [updateQueue],
  );

  const pump = useCallback(async () => {
    if (running.current) return;
    running.current = true;
    // Aucune attente entre la recherche de la prochaine histoire et la libération du verrou :
    // un `enqueue` ne peut pas s'intercaler et rester sans traitement.
    for (;;) {
      const next = queueRef.current.find((item) => item.state === "waiting");
      if (!next) {
        running.current = false;
        break;
      }
      const rootNow = rootRef.current;
      if (!rootNow) {
        // La Flam a disparu : le reste de la file échoue sans rien tenter.
        updateQueue((items) =>
          items.map((item) =>
            item.state === "waiting"
              ? { ...item, state: "failed", error: text.errors.noDevice }
              : item,
          ),
        );
        setAnnouncement(text.panel.announce.failed(next.title, text.errors.noDevice));
        continue;
      }
      const keyAtStart = deviceKeyRef.current;
      sendingUuid.current = next.uuid;
      setProgress(null);
      setItemState(next.uuid, "sending");
      // Après un premier résultat, le suivant s'ajoute : l'échec ne doit pas être couvert.
      const started = text.panel.announce.started(next.title);
      const hasOutcome = queueRef.current.some(
        (item) => item.state !== "waiting" && item.state !== "sending",
      );
      setAnnouncement((previous) =>
        hasOutcome && previous ? `${previous}. ${started}` : started,
      );
      // Histoires en attente au résultat de l'envoi : seules elles peuvent être annulées par ce
      // résultat (un ajout fait pendant la reprise qui suit n'est pas touché).
      const snapshotWaiting = () =>
        new Set(
          queueRef.current.filter((item) => item.state === "waiting").map((i) => i.uuid),
        );
      let waitingAtResult: ReadonlySet<string>;
      // Flam débranchée ou remplacée pendant ou juste après l'envoi (#577).
      let changed = false;
      try {
        const sent = await sendStoryToDevice(
          rootNow,
          next.uuid,
          next.carrier,
          next.replace,
        );
        waitingAtResult = snapshotWaiting();
        changed = sent.device_changed === true;
        if (sent.carrier_readback === false) {
          carrierSession.current = null;
          setItemState(next.uuid, "failed", text.errors.readback);
          setAnnouncement(text.panel.announce.failed(next.title, text.errors.readback));
        } else {
          // Conservation demandée mais échouée : les envois suivants redemandent les clés.
          if (next.carrier?.remember && sent.carrier_stored !== true)
            carrierSession.current = null;
          // Une autre entrée de reprise a échoué, ou la Flam a changé juste après : l'envoi a
          // réussi, mais c'est signalé.
          const warnings = [
            ...(changed ? [text.panel.deviceChangedAfterSend] : []),
            ...(sent.recovery_failures?.length
              ? [text.panel.recoveryFailures(sent.recovery_failures)]
              : []),
          ];
          const warning = warnings.length ? warnings.join(" ") : undefined;
          // Copie de « Mes histoires » (#609) : décidée par le backend, envoi réussi seulement.
          const copy = sent.library_copy ?? null;
          updateQueue((items) =>
            items.map((item) =>
              item.uuid === next.uuid
                ? {
                    ...item,
                    state: "done",
                    error: undefined,
                    warning,
                    pcCopy: undefined,
                    pcCopyError: undefined,
                    pcCopyUnconfirmed: undefined,
                    deviceId: sent.id,
                    ...pcCopyOf(copy),
                  }
                : item,
            ),
          );
          const done = text.panel.announce.done(next.title);
          setAnnouncement(warning ? `${done}. ${warning}` : done);
          if (copy?.status === "kept") {
            // Réglage désactivé : la ligne suffit (rien d'automatique n'était demandé).
            if (copy.unconfirmed && copy.auto)
              onPcCopyChangeRef.current?.(next.title, { status: "kept_unconfirmed" });
          } else if (copy) {
            onPcCopyChangeRef.current?.(next.title, copy);
          }
        }
        bumpReload();
        const gen = reloadGen.current;
        if (sent.carrier_readback !== false) {
          updateQueue((items) =>
            items.map((item) =>
              item.uuid === next.uuid && item.state === "done"
                ? { ...item, reloadGen: gen }
                : item,
            ),
          );
        }
      } catch (caught) {
        waitingAtResult = snapshotWaiting();
        const caughtError = toAppError(caught);
        const failure = { ...caughtError, message: sendErrorMessage(caughtError) };
        // Clés refusées ou introuvables : elles seront redemandées au prochain envoi. Une
        // histoire STUdio refusée en 1.x n'a pas consulté les clés : elles restent valables.
        if (
          next.carrier &&
          failure.kind !== "cancelled" &&
          failure.kind !== LUNII_NEEDS_V2
        )
          carrierSession.current = null;
        if (failure.kind === "cancelled") {
          setItemState(next.uuid, "cancelled");
          setAnnouncement(text.panel.announce.cancelled);
        } else if (failure.kind === DEVICE_CHANGED) {
          setItemState(next.uuid, "failed", failure.message);
          setAnnouncement(text.panel.announce.failed(next.title, failure.message));
          changed = true;
        } else if (
          deviceKeyRef.current === keyAtStart &&
          firmwareWriteBlock(deviceRef.current)
        ) {
          // Refus de la garde du firmware (état connu de la Flam) : les suivantes échoueraient
          // de la même façon.
          setItemState(next.uuid, "failed", failure.message);
          cancelWaiting(waitingAtResult);
          setAnnouncement(text.panel.announce.failed(next.title, failure.message));
        } else {
          setItemState(next.uuid, "failed", failure.message);
          setAnnouncement(text.panel.announce.failed(next.title, failure.message));
          // Un refus de la garde apparu entre-temps (clés devenues invalides, Flam changée…) :
          // l'état est relu.
          if (failure.kind === "validation") retry();
        }
      }
      // Flam débranchée ou remplacée pendant ou juste après l'envoi (refus ou rapport
      // `device_changed`, #577 : la clé d'appareil ne voit pas deux Flam de même libellé sous la
      // même lettre), ou autre Flam détectée : la suite de la file n'est pas envoyée sur une Flam
      // que l'utilisateur n'a pas choisie. Flam simplement absente : la file échoue plus haut
      // (« Flam non détectée »).
      if (
        changed ||
        (deviceKeyRef.current !== null && deviceKeyRef.current !== keyAtStart)
      ) {
        cancelWaiting(waitingAtResult);
      }
      if (changed) {
        // Copie temporaire, cache ou ancienne version peuvent rester sur la Flam branchée :
        // reprise tout de suite (elle supprime une copie inachevée, ne l'installe jamais), puis
        // état relu.
        const rootAfter = rootRef.current;
        if (rootAfter) await recoverDeviceTransfers(rootAfter).catch(() => undefined);
        retry();
      }
      // Le `bt` saisi ne reste pas dans la file au-delà de l'envoi (fait, échoué ou annulé).
      if (next.carrier?.choice.kind === "entered") {
        updateQueue((items) =>
          items.map((item) =>
            item.uuid === next.uuid && item.carrier
              ? { ...item, carrier: undefined }
              : item,
          ),
        );
      }
      sendingUuid.current = null;
      setProgress(null);
    }
    // Plus aucune clé `bt` saisie dans la file (éléments annulés sans envoi compris).
    if (queueRef.current.some((item) => item.carrier?.choice.kind === "entered")) {
      updateQueue((items) =>
        items.map((item) =>
          item.carrier?.choice.kind === "entered" && item.state !== "waiting"
            ? { ...item, carrier: undefined }
            : item,
        ),
      );
    }
    setCancelling(false);
    if (
      queueRef.current.length > 1 &&
      queueRef.current.some((item) => item.state === "done")
    ) {
      setAnnouncement(text.panel.announce.finished);
    }
  }, [setItemState, updateQueue, bumpReload, retry, cancelWaiting]);

  const enqueue = useCallback(
    (request: SendRequest & { carrier?: CarrierSend }) => {
      const active = queueRef.current.some(
        (item) =>
          item.uuid === request.uuid &&
          (item.state === "waiting" || item.state === "sending"),
      );
      if (active) return;
      // Une file terminée est remplacée par une nouvelle ; une file active s'allonge.
      updateQueue((items) => {
        const busy = items.some(
          (item) => item.state === "waiting" || item.state === "sending",
        );
        // Un envoi venu de « Installer et envoyer » s'ajoute à la liste affichée, et un envoi
        // prévu resté en échec y reste lisible jusqu'à sa fermeture.
        const keeps = (item: QueueItem) =>
          busy || request.planned === true || (item.planned && item.state === "failed");
        const base = items.filter((item) => keeps(item) && item.uuid !== request.uuid);
        return [...base, { ...request, state: "waiting" }];
      });
      void pump();
    },
    [pump, updateQueue],
  );

  /** Flam 1.x : les clés carrier sont choisies (et l'avertissement accepté) avant l'envoi. */
  const proceed = useCallback(
    (request: SendRequest) => {
      if (!needsCarrier(deviceRef.current)) {
        enqueue(request);
        return;
      }
      const session = carrierSession.current;
      if (session && session.key === deviceKeyRef.current) {
        enqueue({ ...request, carrier: session.carrier });
      } else {
        setPendingCarrier(request);
      }
    },
    [enqueue],
  );

  /** Exécute `run` une fois l'avertissement « expérimentale » accepté (demandé une seule fois). */
  const withSendWarning = useCallback((run: () => void) => {
    if (getFlamSendWarningAck()) run();
    else setPendingAck({ run });
  }, []);

  const requestSend = useCallback(
    (request: SendRequest) => withSendWarning(() => proceed(request)),
    [withSendWarning, proceed],
  );

  function confirmAck() {
    if (!pendingAck) return;
    setFlamSendWarningAck();
    const { run } = pendingAck;
    setPendingAck(null);
    run();
  }

  // --- Envois prévus à la fin d'une installation (« Installer et envoyer », #609) -----------
  // Rien n'est conservé hors de la mémoire de la fenêtre : après une fermeture ou un
  // rechargement, une installation reprise par le backend ne déclenche aucun envoi.
  const plansRef = useRef<PlannedSend[]>([]);
  const [plans, setPlans] = useState<PlannedSend[]>([]);
  const updatePlans = useCallback((change: (items: PlannedSend[]) => PlannedSend[]) => {
    const next = change(plansRef.current);
    if (next === plansRef.current) return;
    plansRef.current = next;
    setPlans(next);
  }, []);

  const planSend = useCallback(
    (request: PlannedSendRequest, start: () => void) => {
      // Flam du clic : l'avertissement peut rester affiché, et la Flam changer entre-temps.
      const key = deviceKeyRef.current;
      if (key === null) return;
      withSendWarning(() => {
        updatePlans((items) => [
          ...items.filter((item) => item.uuid !== request.uuid),
          {
            ...request,
            deviceKey: key,
            state: "installing",
            size: null,
            lost: deviceKeyRef.current !== key,
          },
        ]);
        setAnnouncement(text.planned.announce.planned(request.title));
        start();
      });
    },
    [withSendWarning, updatePlans],
  );

  const fulfilPlannedSend = useCallback(
    (uuid: string, size: number | null) =>
      updatePlans((items) =>
        items.some((item) => item.uuid === uuid && item.state === "installing")
          ? items.map((item) =>
              item.uuid === uuid ? { ...item, state: "ready", size } : item,
            )
          : items,
      ),
    [updatePlans],
  );

  const dropPlannedSend = useCallback(
    (uuid: string) =>
      updatePlans((items) =>
        items.some((item) => item.uuid === uuid)
          ? items.filter((item) => item.uuid !== uuid)
          : items,
      ),
    [updatePlans],
  );

  // Flam débranchée, remplacée ou rebranchée depuis le clic : l'envoi prévu ne partira pas, même
  // si une Flam de même volume et de même libellé revient (ce peut être une autre : la clé
  // d'appareil ne les distingue pas, #577). L'utilisateur choisira lui-même avec « Envoyer ».
  const plansKey = useRef(deviceKey);
  useEffect(() => {
    if (plansKey.current === deviceKey) return;
    plansKey.current = deviceKey;
    const lost = plansRef.current.filter(
      (item) => !item.lost && item.deviceKey !== deviceKey,
    ).length;
    if (lost === 0) return;
    updatePlans((items) =>
      items.map((item) =>
        !item.lost && item.deviceKey !== deviceKey ? { ...item, lost: true } : item,
      ),
    );
    setAnnouncement(text.planned.announce.lost(lost));
  }, [deviceKey, updatePlans]);

  /** L'envoi prévu ne part pas : ligne en échec dans la file, avec la raison. */
  const refusePlan = useCallback(
    (plan: PlannedSend, message: string) => {
      dropPlannedSend(plan.uuid);
      updateQueue((items) => [
        ...items.filter((item) => item.uuid !== plan.uuid),
        {
          uuid: plan.uuid,
          title: plan.title,
          size: plan.size,
          state: "failed",
          error: message,
          planned: true,
        },
      ]);
      setAnnouncement(text.panel.announce.failed(plan.title, message));
    },
    [dropPlannedSend, updateQueue],
  );

  // Histoires installées : leurs envois partent dans l'ordre des demandes (une histoire installée
  // attend celles demandées avant elle), un par passage, par le chemin de « Envoyer sur la Flam »
  // (mêmes règles, mêmes dialogues, même file). Un dialogue d'envoi affiché suspend la suite : il
  // ne porte qu'une demande à la fois.
  const confirmingSend = pendingAck !== null || pendingCarrier !== null;
  const deviceStories = stories.phase === "ready" ? stories.report : null;
  const storiesError = stories.phase === "error" ? stories.error.message : null;
  useEffect(() => {
    // Une histoire dont l'envoi est déjà abandonné (Flam changée) ne retient pas les suivantes.
    const plan = plans.find((item) => !(item.lost && item.state === "installing"));
    if (!plan || plan.state !== "ready" || confirmingSend) return;
    const block = sendBlock(
      {
        device,
        deviceStories,
        firmwareBlocked,
        removing,
        // Comme le bouton « Envoyer » : en attente, en cours, ou réussi et pas encore relu.
        queuedOrSent: new Set(
          exposedQueue
            .filter(
              (item) =>
                item.state === "waiting" || item.state === "sending" || item.awaitingList,
            )
            .map((item) => item.uuid),
        ),
      },
      plan.uuid,
      plan.size,
      plan.format,
    );
    const changed = plan.lost || device === null || deviceKey !== plan.deviceKey;
    // Retrait en cours sur la Flam, ou liste en cours de lecture : l'envoi attend.
    const waits = block === "removing" || (block === "checking" && storiesError === null);
    if (!changed && waits) return;
    // Décision appliquée hors du corps de l'effet, et seulement si rien n'a changé entre-temps.
    let current = true;
    void Promise.resolve().then(() => {
      if (!current || !plansRef.current.includes(plan)) return;
      if (changed || block === "no_device") {
        refusePlan(plan, text.planned.deviceChanged);
      } else if (block === "checking") {
        // Liste de la Flam illisible : l'envoi ne serait jamais proposé, il n'attend pas.
        refusePlan(plan, text.planned.notSent(storiesError ?? REASON_TEXT.checking));
      } else if (block === "queued") {
        // Déjà dans la file d'envoi (demande faite entre-temps) : rien à ajouter.
        dropPlannedSend(plan.uuid);
      } else if (block !== null) {
        refusePlan(plan, text.planned.notSent(REASON_TEXT[block]));
      } else {
        dropPlannedSend(plan.uuid);
        requestSend({
          uuid: plan.uuid,
          title: plan.title,
          size: plan.size,
          planned: true,
        });
      }
    });
    return () => {
      current = false;
    };
  }, [
    plans,
    confirmingSend,
    device,
    deviceKey,
    deviceStories,
    storiesError,
    firmwareBlocked,
    removing,
    exposedQueue,
    refusePlan,
    dropPlannedSend,
    requestSend,
  ]);

  function confirmCarrier(carrier: CarrierSend) {
    if (!pendingCarrier) return;
    const request = pendingCarrier;
    setPendingCarrier(null);
    // Clés conservées : les envois suivants les relisent ; sinon le même choix est réutilisé,
    // sauf une clé saisie, qui n'est pas gardée en mémoire au-delà de cet envoi.
    carrierSession.current = carrier.remember
      ? {
          key: deviceKeyRef.current,
          carrier: { choice: { kind: "stored" }, remember: false },
        }
      : carrier.choice.kind === "entered"
        ? null
        : { key: deviceKeyRef.current, carrier };
    enqueue({ ...request, carrier });
  }

  const forgetCarrier = useCallback(async () => {
    const rootNow = rootRef.current;
    if (!rootNow) return 0;
    carrierSession.current = null;
    return forgetCarrierKeys(rootNow);
  }, []);

  const cancelQueue = useCallback(() => {
    // Les envois prévus sont oubliés aussi (les installations continuent) : rien ne repart seul
    // après une annulation.
    updatePlans((items) => (items.length > 0 ? [] : items));
    updateQueue((items) =>
      items.map((item) =>
        item.state === "waiting" ? { ...item, state: "cancelled" } : item,
      ),
    );
    if (!running.current) return;
    setCancelling(true);
    void (async () => {
      for (let attempt = 0; attempt < CANCEL_RETRIES && running.current; attempt += 1) {
        try {
          if (await cancelDeviceTransfer()) return;
        } catch {
          setCancelling(false);
          return;
        }
        await new Promise((resolve) => setTimeout(resolve, CANCEL_RETRY_MS));
      }
    })();
  }, [updateQueue, updatePlans]);

  const removeFromQueue = useCallback(
    (uuid: string) =>
      updateQueue((items) =>
        items.filter((item) => !(item.uuid === uuid && item.state === "waiting")),
      ),
    [updateQueue],
  );

  // Histoire supprimée de la bibliothèque : sa ligne d'envoi réussie n'a plus d'objet. Les échecs
  // et annulations restent (message), comme les envois retirés du PC par cette file (#609 : la
  // ligne dit que l'histoire reste sur la Flam). Sans effet tant que `libraryUuids` n'est pas
  // connu.
  useEffect(() => {
    if (!libraryUuids) return;
    updateQueue((items) => {
      const kept = items.filter(
        (item) =>
          item.state !== "done" ||
          item.pcCopy === "removed" ||
          item.pcCopy === "removing" ||
          libraryUuids.has(item.uuid),
      );
      return kept.length === items.length ? items : kept;
    });
  }, [libraryUuids, updateQueue]);

  const setPcCopy = useCallback(
    (uuid: string, pcCopy: PcCopyState, pcCopyError?: string) =>
      updateQueue((items) =>
        items.map((item) =>
          item.uuid === uuid && item.state === "done"
            ? { ...item, pcCopy, pcCopyError }
            : item,
        ),
      ),
    [updateQueue],
  );

  const removeFromPc = useCallback(
    async (uuid: string) => {
      const item = queueRef.current.find(
        (entry) => entry.uuid === uuid && entry.state === "done",
      );
      if (!item || (item.pcCopy !== "kept" && item.pcCopy !== "failed")) return;
      // Le backend refuse pendant un envoi (verrou de la Flam) : pas d'appel inutile.
      if (running.current) return;
      const rootNow = rootRef.current;
      if (!rootNow) {
        setPcCopy(uuid, "refused", text.panel.pcCopy.noDevice);
        return;
      }
      setPcCopy(uuid, "removing");
      try {
        await removeSentFromLibrary(rootNow, uuid);
        setPcCopy(uuid, "removed");
        onPcCopyChangeRef.current?.(item.title, { status: "removed" });
      } catch (caught) {
        // Refus définitif (autre Flam, histoire plus sur la Flam ou changée, envoi inconnu) : le
        // bouton n'est plus proposé. Refus passager (lecture, installation, Flam occupée…) : le
        // bouton reste, avec le message.
        const error = toAppError(caught);
        const message = error.message;
        setPcCopy(
          uuid,
          DEFINITIVE_REFUSALS.has(error.kind) ? "refused" : "failed",
          message,
        );
        onPcCopyChangeRef.current?.(item.title, { status: "remove_failed", message });
      }
    },
    [setPcCopy],
  );

  const dismissQueue = useCallback(() => {
    if (running.current) return;
    updateQueue(() => []);
    setAnnouncement("");
  }, [updateQueue]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    onTransferProgress((event) => {
      if (event.uuid !== sendingUuid.current || event.total <= 0) return;
      setProgress({
        uuid: event.uuid,
        phase: event.phase,
        fraction: Math.min(1, event.done / event.total),
      });
    })
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      // Le retrait de l'écouteur est asynchrone et peut échouer à la fermeture de la fenêtre.
      void Promise.resolve()
        .then(() => unlisten?.())
        .catch(() => undefined);
    };
  }, []);

  // --- Retrait et éjection -----------------------------------------------------------------
  const removeFromDevice = useCallback(
    async (uuid: string, expectedKey: string | null) => {
      const rootNow = rootRef.current;
      if (!rootNow || deviceKeyRef.current !== expectedKey)
        throw { kind: "not_found", message: text.errors.noDevice };
      setRemoving(true);
      // Histoire retirée de la Flam : « Retirer du PC » ne vaut plus pour elle, quelle que soit
      // l'issue du retrait (le backend l'oublie aussi, #609).
      clearPcCopy((item) => sameStory(item.deviceId, uuid));
      try {
        const report = await removeDeviceStory(rootNow, uuid, true);
        bumpReload();
        return { cleanupPending: report.cleanup_pending };
      } finally {
        setRemoving(false);
      }
    },
    [bumpReload, clearPcCopy],
  );

  const reorderOnDevice = useCallback(
    async (order: string[], expectedKey: string | null) => {
      const rootNow = rootRef.current;
      if (!rootNow || deviceKeyRef.current !== expectedKey)
        throw { kind: "not_found", message: text.errors.noDevice };
      try {
        return await reorderDeviceStories(rootNow, order);
      } finally {
        // Succès ou refus : la liste affichée suit l'index réel de la Flam.
        bumpReload();
      }
    },
    [bumpReload],
  );

  const [ejecting, setEjecting] = useState(false);
  const eject = useCallback(async () => {
    const rootNow = rootRef.current;
    if (!rootNow) return;
    setEjecting(true);
    setEjectNote(null);
    try {
      const outcome = await ejectDevice(rootNow);
      setEjectNote({
        tone: outcome.status === "ejected" ? "success" : "warning",
        message: outcome.message,
      });
      retry();
    } catch (caught) {
      setEjectNote({
        tone: "danger",
        message: text.eject.failed(toAppError(caught).message),
      });
    } finally {
      setEjecting(false);
    }
  }, [retry]);

  const busy = queue.some((item) => item.state === "waiting" || item.state === "sending");
  const api: DeviceApi = useMemo(
    () => ({
      status,
      error,
      retry,
      retryAnswered: answered,
      device,
      deviceKey,
      recovered,
      recoveryNote,
      stories,
      reloadStories,
      firmwareBlocked,
      canMaintain: writable,
      queue: exposedQueue,
      progress,
      cancelling,
      busy,
      requestSend,
      confirmingSend,
      plannedSends: plans,
      planSend,
      fulfilPlannedSend,
      dropPlannedSend,
      forgetCarrier,
      cancelQueue,
      removeFromQueue,
      dismissQueue,
      removeFromPc,
      removeFromDevice,
      reorderOnDevice,
      eject,
      ejecting,
      removing,
      ejectNote,
      announcement,
    }),
    [
      status,
      error,
      retry,
      answered,
      device,
      deviceKey,
      recovered,
      recoveryNote,
      stories,
      reloadStories,
      firmwareBlocked,
      writable,
      exposedQueue,
      progress,
      cancelling,
      busy,
      requestSend,
      confirmingSend,
      plans,
      planSend,
      fulfilPlannedSend,
      dropPlannedSend,
      forgetCarrier,
      cancelQueue,
      removeFromQueue,
      dismissQueue,
      removeFromPc,
      removeFromDevice,
      reorderOnDevice,
      eject,
      ejecting,
      removing,
      ejectNote,
      announcement,
    ],
  );

  return (
    <DeviceContext.Provider value={api}>
      {children}
      <Dialog
        open={pendingAck !== null}
        onClose={() => setPendingAck(null)}
        role="alertdialog"
        title={text.experimental.title}
        description={text.experimental.description}
        initialFocus={cancelButton}
        footer={
          <>
            <Button ref={cancelButton} onClick={() => setPendingAck(null)}>
              {t.common.cancel}
            </Button>
            <Button variant="primary" icon="install" onClick={confirmAck}>
              {text.experimental.confirm}
            </Button>
          </>
        }
      >
        <Banner tone="warning">{text.experimental.body}</Banner>
      </Dialog>
      {pendingCarrier && root && (
        <CarrierDialog
          root={root}
          onCancel={() => setPendingCarrier(null)}
          onConfirm={confirmCarrier}
        />
      )}
    </DeviceContext.Provider>
  );
}
