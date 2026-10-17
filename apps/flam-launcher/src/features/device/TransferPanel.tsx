import { useEffect, useId, useRef } from "react";
import { Badge, Banner, Button, IconButton, ProgressBar } from "../../components";
import type { BadgeTone } from "../../components";
import { t } from "../../i18n";
import { useDevice, type QueueItem, type QueueState } from "./DeviceContext";
import { EjectButton } from "./EjectButton";
import { estimateMinutes } from "./sendRules";
import "./TransferPanel.css";

const text = t.deviceTransfer;

const STATE_TONES: Record<QueueState, BadgeTone> = {
  waiting: "neutral",
  sending: "secondary",
  done: "success",
  failed: "danger",
  cancelled: "neutral",
};

/** Poids d'une histoire dans la progression globale : sa taille, ou 1 si une taille manque. */
function weights(queue: QueueItem[]): number[] {
  const known = queue.every((item) => item.size !== null);
  return queue.map((item) => (known ? Math.max(item.size ?? 1, 1) : 1));
}

/** Panneau d'envoi sur la Flam : progression globale et courante, annulation, fin de file. */
export function TransferPanel() {
  const {
    queue,
    progress,
    cancelling,
    busy,
    cancelQueue,
    removeFromQueue,
    dismissQueue,
    announcement,
  } = useDevice();
  const titleId = useId();
  const headingRef = useRef<HTMLHeadingElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const wasBusy = useRef(false);

  // Fin de file : le bouton Annuler disparaît, le focus passe au titre du panneau.
  useEffect(() => {
    if (wasBusy.current && !busy && document.activeElement === document.body) {
      headingRef.current?.focus();
    }
    wasBusy.current = busy;
  });

  // Annonces de la file (début, fin, échec), sans pourcentage : région vivante toujours présente.
  const live = (
    <p role="status" className="fl-visually-hidden">
      {announcement}
    </p>
  );
  if (queue.length === 0) return live;

  const weight = weights(queue);
  const total = weight.reduce((a, b) => a + b, 0);
  const current = queue.findIndex((item) => item.state === "sending");
  const currentItem = current >= 0 ? queue[current] : undefined;
  const finished = queue.filter(
    (item) => item.state !== "waiting" && item.state !== "sending",
  ).length;
  const fraction =
    progress && currentItem?.uuid === progress.uuid ? progress.fraction : 0;
  let doneWeight = 0;
  queue.forEach((item, index) => {
    if (item.state !== "waiting" && item.state !== "sending") {
      doneWeight += weight[index] ?? 0;
    }
  });
  if (current >= 0) doneWeight += (weight[current] ?? 0) * fraction;
  const percent = total > 0 ? (doneWeight / total) * 100 : 0;

  let remaining: number | null = 0;
  for (const item of queue) {
    if (item.state !== "waiting" && item.state !== "sending") continue;
    if (item.size === null) {
      remaining = null;
      break;
    }
    remaining += item.state === "sending" ? item.size * (1 - fraction) : item.size;
  }
  const minutes = estimateMinutes(remaining);
  const anyDone = queue.some((item) => item.state === "done");

  return (
    <>
      {live}
      <section className="fl-transfer" aria-labelledby={titleId}>
        <h2 id={titleId} ref={headingRef} tabIndex={-1} className="fl-transfer__title">
          {text.panel.title}
        </h2>
        {busy && (
          <div className="fl-transfer__progress">
            <p className="fl-transfer__line">
              {text.panel.overall(Math.min(finished + 1, queue.length), queue.length)}
            </p>
            <ProgressBar value={percent} label={text.panel.overallLabel} showValue />
            {currentItem && (
              <>
                <p className="fl-transfer__line">
                  {text.panel.current(currentItem.title)}
                </p>
                <ProgressBar
                  value={fraction * 100}
                  label={text.panel.currentLabel(currentItem.title)}
                  showValue
                />
                {progress && (
                  <p className="fl-transfer__hint">{text.panel.phase[progress.phase]}</p>
                )}
              </>
            )}
            {minutes !== null && (
              <p className="fl-transfer__hint">{text.panel.estimate(minutes)}</p>
            )}
            <Button
              ref={cancelRef}
              icon="close"
              className="fl-send__button"
              aria-disabled={cancelling || undefined}
              onClick={() => {
                if (!cancelling) cancelQueue();
              }}
            >
              {cancelling ? text.panel.cancelling : text.panel.cancel}
            </Button>
          </div>
        )}
        <ul className="fl-transfer__list" aria-label={text.panel.listLabel}>
          {queue.map((item) => (
            <li key={item.uuid} className="fl-transfer__item">
              <div className="fl-transfer__item-head">
                <span className="fl-transfer__name">{item.title}</span>
                {item.state === "waiting" && (
                  <IconButton
                    icon="close"
                    size="sm"
                    label={text.panel.removeFromQueue(item.title)}
                    onClick={() => removeFromQueue(item.uuid)}
                  />
                )}
              </div>
              <Badge
                tone={STATE_TONES[item.state]}
                icon={item.state === "failed" ? "alert" : undefined}
              >
                {text.panel.state[item.state]}
              </Badge>
              {item.error && <p className="fl-transfer__error">{item.error}</p>}
            </li>
          ))}
        </ul>
        {!busy && (
          <>
            {anyDone && (
              <Banner tone="success" title={text.finished.title} role="status">
                {text.finished.restart}
              </Banner>
            )}
            <div className="fl-transfer__actions">
              {anyDone && <EjectButton />}
              <Button
                variant="ghost"
                onClick={() => {
                  dismissQueue();
                  document.getElementById("fl-main")?.focus();
                }}
              >
                {text.panel.dismiss}
              </Button>
            </div>
          </>
        )}
      </section>
    </>
  );
}
