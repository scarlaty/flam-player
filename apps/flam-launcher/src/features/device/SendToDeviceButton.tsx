import { useId, useMemo } from "react";
import { Button, type ButtonVariant } from "../../components";
import { t } from "../../i18n";
import { useDevice } from "./DeviceContext";
import { sendBlock, type SendBlock } from "./sendRules";
import "./SendToDeviceButton.css";

const text = t.deviceTransfer;

const REASON_TEXT: Record<SendBlock, string> = {
  queued: text.reasons.queued,
  no_device: text.reasons.noDevice,
  firmware: text.reasons.firmware,
  checking: text.reasons.checking,
  present: text.reasons.present,
  no_space: text.reasons.noSpace,
};

export interface SendToDeviceButtonProps {
  uuid: string;
  title: string;
  /** Taille du `.plain.pk` en octets (`null` : inconnue). */
  size: number | null;
  variant?: ButtonVariant;
}

/** Raison de blocage de l'envoi de `uuid` d'après l'état de la Flam et de la file. */
export function useSendBlock(uuid: string, size: number | null): SendBlock | null {
  const { device, stories, firmwareBlocked, queue } = useDevice();
  const report = stories.phase === "ready" ? stories.report : null;
  const queuedOrSent = useMemo(
    () =>
      new Set(
        queue
          .filter((item) => item.state !== "failed" && item.state !== "cancelled")
          .map((item) => item.uuid),
      ),
    [queue],
  );
  return sendBlock(
    { device, deviceStories: report, firmwareBlocked, queuedOrSent },
    uuid,
    size,
  );
}

/**
 * « Envoyer sur la Flam ». Quand l'envoi est impossible, le bouton reste focalisable
 * (`aria-disabled`), ne fait rien et sa raison est écrite à côté, reliée par `aria-describedby`.
 */
export function SendToDeviceButton({
  uuid,
  title,
  size,
  variant = "secondary",
}: SendToDeviceButtonProps) {
  const { requestSend } = useDevice();
  const block = useSendBlock(uuid, size);
  const reasonId = useId();
  return (
    <span className="fl-send">
      <Button
        size="sm"
        variant={variant}
        icon="device"
        className="fl-send__button"
        aria-disabled={block !== null}
        aria-describedby={block ? reasonId : undefined}
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
    </span>
  );
}
