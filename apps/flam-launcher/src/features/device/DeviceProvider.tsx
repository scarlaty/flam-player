import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  cancelDeviceTransfer,
  ejectDevice,
  forgetCarrierKeys,
  listDeviceStories,
  needsCarrier,
  onTransferProgress,
  recoverDeviceTransfers,
  removeDeviceStory,
  sendStoryToDevice,
  type CarrierSend,
  type DeviceSummary,
} from "../../api/device";
import { toAppError } from "../../api/error";
import { Banner, Button, Dialog } from "../../components";
import { t } from "../../i18n";
import { getFlamSendWarningAck, setFlamSendWarningAck } from "../settings/preferences";
import {
  DeviceContext,
  type DeviceApi,
  type EjectNote,
  type QueueItem,
  type SendRequest,
  type StoriesState,
  type TransferProgressView,
} from "./DeviceContext";
import { CarrierDialog } from "./CarrierDialog";
import { useDeviceStatus } from "./useDeviceStatus";

const text = t.deviceTransfer;

/** Intervalle des tentatives d'annulation tant que le transfert n'a pas pris le drapeau. */
const CANCEL_RETRY_MS = 200;
const CANCEL_RETRIES = 15;

/** Refus du backend pour un firmware non pris en charge (3.x et suivants, `.mdf` illisible). */
const FIRMWARE_ERROR = /firmware de la Flam non pris en charge/i;

export interface DeviceProviderProps {
  children: ReactNode;
  /** Pas de scrutation (player incrusté). */
  paused?: boolean;
  /** Délai entre deux scrutations (défaut : celui de `useDeviceStatus`). */
  intervalMs?: number;
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
}: DeviceProviderProps) {
  const { status, error, retry } = useDeviceStatus({ enabled: !paused, intervalMs });
  const device = status?.detected ? status.device : null;
  const root = device?.root ?? null;
  const deviceKey = device ? `${device.root}\u0000${device.volume_label}` : null;
  const known = status !== null;

  const rootRef = useRef<string | null>(null);
  const deviceKeyRef = useRef<string | null>(null);
  const deviceRef = useRef<DeviceSummary | null>(null);
  useEffect(() => {
    rootRef.current = root;
    deviceKeyRef.current = deviceKey;
    deviceRef.current = device;
  });

  // Reprise : une fois par branchement (changement de volume ou retour après une absence).
  const startedKey = useRef<string | null>(null);
  const [recoveredKey, setRecoveredKey] = useState<string | null>(null);
  const [recoveryNote, setRecoveryNote] = useState<EjectNote | null>(null);
  const [stories, setStories] = useState<StoriesState>({ phase: "loading" });
  const [reload, setReload] = useState(0);
  const [firmwareKey, setFirmwareKey] = useState<string | null>(null);
  const [ejectNote, setEjectNote] = useState<EjectNote | null>(null);

  // Nouvelle Flam, retour après une absence ou perte : l'état lié à la détection repart de zéro
  // (ajustement pendant le rendu, sans effet).
  const [seenKey, setSeenKey] = useState<string | null>(null);
  if (known && seenKey !== deviceKey) {
    setSeenKey(deviceKey);
    setRecoveredKey(null);
    setRecoveryNote(null);
    setStories({ phase: "loading" });
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
        if (startedKey.current === key) setRecoveredKey(key);
      });
  }, [known, deviceKey, root]);

  const recovered = deviceKey !== null && recoveredKey === deviceKey;

  useEffect(() => {
    if (!recovered) return;
    let active = true;
    listDeviceStories()
      .then((report) => {
        if (!active) return;
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
        if (active) setStories({ phase: "error", error: toAppError(caught) });
      });
    return () => {
      active = false;
    };
  }, [recovered, deviceKey, reload]);

  const reloadStories = useCallback((options?: { quiet?: boolean }) => {
    if (!options?.quiet) setStories({ phase: "loading" });
    setReload((n) => n + 1);
  }, []);

  // --- File d'envoi ---------------------------------------------------------------------
  const queueRef = useRef<QueueItem[]>([]);
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const [progress, setProgress] = useState<TransferProgressView | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const [announcement, setAnnouncement] = useState("");
  const running = useRef(false);
  const sendingUuid = useRef<string | null>(null);
  const [pendingAck, setPendingAck] = useState<SendRequest | null>(null);
  // Mode carrier (Flam 1.x) : demande en attente du choix des clés, et choix retenu pour la
  // Flam branchée pendant cette session.
  const [pendingCarrier, setPendingCarrier] = useState<SendRequest | null>(null);
  const carrierSession = useRef<{ key: string | null; carrier: CarrierSend } | null>(
    null,
  );
  const cancelButton = useRef<HTMLButtonElement>(null);

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
      try {
        const sent = await sendStoryToDevice(
          rootNow,
          next.uuid,
          next.carrier,
          next.replace === true,
        );
        if (sent.carrier_readback === false) {
          carrierSession.current = null;
          setItemState(next.uuid, "failed", text.errors.readback);
          setAnnouncement(text.panel.announce.failed(next.title, text.errors.readback));
        } else {
          // Conservation demandée mais échouée : les envois suivants redemandent les clés.
          if (next.carrier?.remember && sent.carrier_stored !== true)
            carrierSession.current = null;
          setItemState(next.uuid, "done");
          setAnnouncement(text.panel.announce.done(next.title));
        }
        setReload((n) => n + 1);
      } catch (caught) {
        const failure = toAppError(caught);
        // Clés refusées ou introuvables : elles seront redemandées au prochain envoi.
        if (next.carrier && failure.kind !== "cancelled") carrierSession.current = null;
        if (failure.kind === "cancelled") {
          setItemState(next.uuid, "cancelled");
          setAnnouncement(text.panel.announce.cancelled);
        } else if (FIRMWARE_ERROR.test(failure.message)) {
          setFirmwareKey(keyAtStart);
          setItemState(next.uuid, "failed", text.errors.firmware);
          // Inutile d'essayer les suivantes : elles échoueraient de la même façon.
          updateQueue((items) =>
            items.map((item) =>
              item.state === "waiting" ? { ...item, state: "cancelled" } : item,
            ),
          );
          setAnnouncement(text.panel.announce.failed(next.title, text.errors.firmware));
        } else {
          setItemState(next.uuid, "failed", failure.message);
          setAnnouncement(text.panel.announce.failed(next.title, failure.message));
        }
      }
      sendingUuid.current = null;
      setProgress(null);
    }
    setCancelling(false);
    if (
      queueRef.current.length > 1 &&
      queueRef.current.some((item) => item.state === "done")
    ) {
      setAnnouncement(text.panel.announce.finished);
    }
  }, [setItemState, updateQueue]);

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
        const base = busy ? items.filter((item) => item.uuid !== request.uuid) : [];
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

  const requestSend = useCallback(
    (request: SendRequest) => {
      if (getFlamSendWarningAck()) proceed(request);
      else setPendingAck(request);
    },
    [proceed],
  );

  function confirmAck() {
    if (!pendingAck) return;
    setFlamSendWarningAck();
    const request = pendingAck;
    setPendingAck(null);
    proceed(request);
  }

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
  }, [updateQueue]);

  const removeFromQueue = useCallback(
    (uuid: string) =>
      updateQueue((items) =>
        items.filter((item) => !(item.uuid === uuid && item.state === "waiting")),
      ),
    [updateQueue],
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
      const report = await removeDeviceStory(rootNow, uuid, true);
      setReload((n) => n + 1);
      return { cleanupPending: report.cleanup_pending };
    },
    [],
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
      device,
      deviceKey,
      recovered,
      recoveryNote,
      stories,
      reloadStories,
      firmwareBlocked: firmwareKey !== null && firmwareKey === deviceKey,
      queue,
      progress,
      cancelling,
      busy,
      requestSend,
      confirmingSend: pendingAck !== null || pendingCarrier !== null,
      forgetCarrier,
      cancelQueue,
      removeFromQueue,
      dismissQueue,
      removeFromDevice,
      eject,
      ejecting,
      ejectNote,
      announcement,
    }),
    [
      status,
      error,
      retry,
      device,
      deviceKey,
      recovered,
      recoveryNote,
      stories,
      reloadStories,
      firmwareKey,
      queue,
      progress,
      cancelling,
      busy,
      requestSend,
      pendingAck,
      pendingCarrier,
      forgetCarrier,
      cancelQueue,
      removeFromQueue,
      dismissQueue,
      removeFromDevice,
      eject,
      ejecting,
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
