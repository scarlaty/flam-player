import { useId, useMemo } from "react";
import { Button, type ButtonVariant } from "../../components";
import { t } from "../../i18n";
import { useDevice } from "./DeviceContext";
import {
  findOnDevice,
  replaceBlock,
  sendBlock,
  type SendBlock,
  type SendContext,
} from "./sendRules";
import "./SendToDeviceButton.css";

const text = t.deviceTransfer;

const REASON_TEXT: Record<SendBlock, string> = {
  queued: text.reasons.queued,
  no_device: text.reasons.noDevice,
  firmware: text.reasons.firmware,
  checking: text.reasons.checking,
  present: text.reasons.present,
  no_space: text.reasons.noSpace,
  removing: text.reasons.removing,
};

export interface SendToDeviceButtonProps {
  uuid: string;
  title: string;
  /** Taille du `.plain.pk` en octets (`null` : inconnue). */
  size: number | null;
  variant?: ButtonVariant;
  /**
   * La copie de bibliothèque est plus ancienne que sa source dossier (#288) : signalé à côté du
   * bouton, sans bloquer l'envoi.
   */
  stale?: boolean;
}

/** État de la Flam et de la file, pour les règles d'envoi. */
function useSendContext(): SendContext {
  const { device, stories, firmwareBlocked, queue, removing } = useDevice();
  const report = stories.phase === "ready" ? stories.report : null;
  // Seuls les envois en attente ou en cours bloquent : une tâche terminée (réussie, en échec ou
  // annulée) ne bloque jamais un nouvel envoi. Une histoire réussie y figure déjà (« Déjà sur la
  // Flam », relue après l'envoi) ; si elle en a été retirée, elle peut repartir.
  const queuedOrSent = useMemo(
    () =>
      new Set(
        queue
          .filter(
            (item) =>
              item.state === "waiting" || item.state === "sending" || item.awaitingList,
          )
          .map((item) => item.uuid),
      ),
    [queue],
  );
  return {
    device,
    deviceStories: report,
    firmwareBlocked,
    removing,
    queuedOrSent,
  };
}

/** Raison de blocage de l'envoi de `uuid` d'après l'état de la Flam et de la file. */
export function useSendBlock(uuid: string, size: number | null): SendBlock | null {
  return sendBlock(useSendContext(), uuid, size);
}

/** Identifiants non vides séparés par des espaces (`aria-describedby`), `undefined` sinon. */
function describedBy(...ids: (string | false)[]): string | undefined {
  return ids.filter((id): id is string => id !== false).join(" ") || undefined;
}

/**
 * « Envoyer sur la Flam ». Quand l'envoi est impossible, le bouton reste focalisable
 * (`aria-disabled`), ne fait rien et sa raison est écrite à côté, reliée par `aria-describedby`.
 * Une histoire déjà sur la Flam (non masquée) peut y être **remplacée** (« Remplacer sur la
 * Flam », même à version égale : après une mise à jour depuis la source, #288).
 */
export function SendToDeviceButton({
  uuid,
  title,
  size,
  variant = "secondary",
  stale = false,
}: SendToDeviceButtonProps) {
  const { requestSend } = useDevice();
  const context = useSendContext();
  const block = sendBlock(context, uuid, size);
  const replace = block === "present" ? replaceBlock(context, uuid, size) : "absent";
  // UUID réel de l'histoire sur la Flam (celui de `uuid.bin`, pas celui de la bibliothèque).
  const target = findOnDevice(context.deviceStories, uuid)?.uuid;
  const reasonId = useId();
  const staleId = useId();
  const replaceReasonId = useId();
  return (
    <span className="fl-send">
      <Button
        size="sm"
        variant={variant}
        icon="device"
        className="fl-send__button"
        aria-disabled={block !== null}
        aria-describedby={describedBy(block !== null && reasonId, stale && staleId)}
        aria-label={text.forStory(text.send, title)}
        onClick={() => {
          if (block === null) requestSend({ uuid, title, size });
        }}
      >
        {text.send}
      </Button>
      {block && (
        <span id={reasonId} className="fl-send__reason">
          {REASON_TEXT[block]}
        </span>
      )}
      {replace !== "absent" && (
        <>
          <Button
            size="sm"
            variant={variant}
            icon="refresh"
            className="fl-send__button"
            aria-disabled={replace !== null}
            aria-describedby={describedBy(
              replace !== null && replaceReasonId,
              stale && staleId,
            )}
            aria-label={text.forStory(text.replace, title)}
            onClick={() => {
              if (replace === null && target)
                requestSend({ uuid, title, size, replace: target });
            }}
          >
            {text.replace}
          </Button>
          {replace && (
            <span id={replaceReasonId} className="fl-send__reason">
              {REASON_TEXT[replace]}
            </span>
          )}
        </>
      )}
      {stale && (
        <span id={staleId} className="fl-send__reason">
          {text.staleCopy}
        </span>
      )}
    </span>
  );
}
