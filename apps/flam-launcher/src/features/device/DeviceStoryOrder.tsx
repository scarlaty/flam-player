import { useLayoutEffect, useRef, useState } from "react";
import type { DeviceStory } from "../../api/device";
import { toAppError, type AppError } from "../../api/error";
import { Button, IconButton, useToast } from "../../components";
import { t } from "../../i18n";
import { ErrorBanner } from "../ErrorBanner";
import { moveItem } from "../sources/SourcesScreen";
import { useDevice } from "./DeviceContext";
import "./DeviceStoryOrder.css";
import "./SendToDeviceButton.css";

const text = t.deviceScreen.order;

export interface DeviceStoryOrderProps {
  /** Histoires visibles, dans l'ordre de l'index `list` (toutes : liste non tronquée). */
  stories: DeviceStory[];
  /** Nom affiché d'une histoire. */
  nameOf: (story: DeviceStory) => string;
  /** Des histoires masquées ou non listées existent (hors de l'ordre). */
  hasOthers: boolean;
  /** Un envoi est en cours : rien n'est modifiable. */
  busy: boolean;
  /** Fin de l'édition (enregistrée ou annulée). */
  onClose: () => void;
}

type MoveDir = "top" | "up" | "down" | "bottom";

/** Boutons de repli pour garder le focus quand celui de l'action est désactivé à l'arrivée. */
const FOCUS_FALLBACK: Record<MoveDir, MoveDir[]> = {
  top: ["top", "down", "bottom", "up"],
  up: ["up", "down", "top", "bottom"],
  down: ["down", "up", "bottom", "top"],
  bottom: ["bottom", "up", "top", "down"],
};

const sameSet = (a: readonly string[], b: readonly string[]) =>
  a.length === b.length && b.every((uuid) => a.includes(uuid));

/**
 * Édition de l'ordre des histoires visibles de la Flam : glisser-déposer ou boutons Tout en haut /
 * Monter / Descendre / Tout en bas (Début / Fin au clavier), puis « Enregistrer l'ordre » (une seule écriture de l'index). Rien n'est écrit
 * avant l'enregistrement ; « Annuler » abandonne les déplacements.
 */
export function DeviceStoryOrder({
  stories,
  nameOf,
  hasOthers,
  busy,
  onClose,
}: DeviceStoryOrderProps) {
  const { reorderOnDevice, deviceKey } = useDevice();
  const toast = useToast();
  const base = stories.map((story) => story.uuid);
  const [keyAtOpen] = useState(deviceKey);
  const [order, setOrder] = useState<string[]>(base);
  const [seenBase, setSeenBase] = useState(base);
  const [dragId, setDragId] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const [announcement, setAnnouncement] = useState("");
  const listRef = useRef<HTMLOListElement>(null);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const pendingFocus = useRef<{ uuid: string; dir: MoveDir } | null>(null);

  // Liste relue entre-temps (envoi, retrait, refus) : un ordre qui ne porte plus sur les mêmes
  // histoires repart de celui de la Flam (ajustement pendant le rendu).
  if (seenBase.join() !== base.join()) {
    setSeenBase(base);
    if (!sameSet(order, base)) {
      setOrder(base);
      setAnnouncement(text.reset);
    }
  }

  useLayoutEffect(() => headingRef.current?.focus(), []);

  // Garde le focus sur le bouton de déplacement de l'histoire déplacée au clavier.
  useLayoutEffect(() => {
    const pending = pendingFocus.current;
    if (!pending || !listRef.current) return;
    pendingFocus.current = null;
    const find = (dir: string) =>
      listRef.current?.querySelector<HTMLButtonElement>(
        `[data-uuid="${pending.uuid}"][data-move="${dir}"]`,
      );
    for (const dir of FOCUS_FALLBACK[pending.dir]) {
      const button = find(dir);
      if (button && !button.disabled) {
        button.focus();
        return;
      }
    }
  }, [order]);

  const byUuid = new Map(stories.map((story) => [story.uuid, story]));
  const locked = saving || busy;
  const dirty = order.join() !== base.join();

  function apply(next: string[], uuid: string) {
    setOrder(next);
    setError(null);
    const story = byUuid.get(uuid);
    if (story) {
      setAnnouncement(text.moved(nameOf(story), next.indexOf(uuid) + 1, next.length));
    }
  }

  function moveTo(uuid: string, dir: MoveDir) {
    if (locked) return;
    const from = order.indexOf(uuid);
    if (from < 0) return;
    const to =
      dir === "top"
        ? 0
        : dir === "bottom"
          ? order.length - 1
          : from + (dir === "up" ? -1 : 1);
    if (to < 0 || to >= order.length || to === from) return;
    pendingFocus.current = { uuid, dir };
    apply(moveItem(order, from, to), uuid);
  }

  function dropOn(targetUuid: string) {
    const uuid = dragId;
    setDragId(null);
    if (!uuid || uuid === targetUuid || locked) return;
    const from = order.indexOf(uuid);
    const to = order.indexOf(targetUuid);
    if (from < 0 || to < 0) return;
    apply(moveItem(order, from, to), uuid);
  }

  async function save() {
    if (!dirty || locked) return;
    setSaving(true);
    setError(null);
    try {
      const report = await reorderOnDevice(order, keyAtOpen);
      toast.show({
        message: report.changed ? text.saved : text.unchanged,
        tone: "success",
      });
      onClose();
    } catch (caught) {
      setSaving(false);
      setError(toAppError(caught));
    }
  }

  return (
    <section className="fl-device-order" aria-labelledby="fl-device-order-title">
      <h3
        id="fl-device-order-title"
        ref={headingRef}
        tabIndex={-1}
        className="fl-device-order__title"
      >
        {text.title}
      </h3>
      <p id="fl-device-order-help" className="fl-device-order__help">
        {text.help}
      </p>
      {hasOthers && <p className="fl-device-order__help">{text.othersNote}</p>}
      <p role="status" className="fl-visually-hidden">
        {saving ? text.saving : announcement}
      </p>
      {error && <ErrorBanner error={{ ...error, message: text.failed(error.message) }} />}
      <ol
        ref={listRef}
        className="fl-device-order__rows"
        aria-label={text.listLabel}
        aria-describedby="fl-device-order-help"
      >
        {order.map((uuid, index) => {
          const story = byUuid.get(uuid);
          if (!story) return null;
          const name = nameOf(story);
          return (
            <li
              key={uuid}
              className={
                "fl-device-order__row" +
                (locked ? "" : " fl-device-order__row--draggable") +
                (dragId === uuid ? " fl-device-order__row--dragging" : "")
              }
              onKeyDown={(event) => {
                // Début / Fin sur la ligne de l'élément focalisé (bouton ou ligne).
                if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey)
                  return;
                if (event.key !== "Home" && event.key !== "End") return;
                event.preventDefault();
                moveTo(uuid, event.key === "Home" ? "top" : "bottom");
              }}
              draggable={!locked}
              onDragStart={(event) => {
                if (locked) return;
                if (event.dataTransfer) {
                  event.dataTransfer.effectAllowed = "move";
                  event.dataTransfer.setData("text/plain", uuid);
                }
                setDragId(uuid);
              }}
              onDragOver={(event) => {
                if (dragId && !locked) event.preventDefault();
              }}
              onDrop={(event) => {
                event.preventDefault();
                dropOn(uuid);
              }}
              onDragEnd={() => setDragId(null)}
            >
              <span className="fl-device-order__grip" aria-hidden="true" />
              <span className="fl-device-order__position" aria-hidden="true">
                {text.position(index + 1)}
              </span>
              <span className="fl-device-order__name">{name}</span>
              <span className="fl-device-order__moves">
                <IconButton
                  icon="arrowTop"
                  size="sm"
                  label={text.moveTop(name)}
                  data-uuid={uuid}
                  data-move="top"
                  disabled={locked || index === 0}
                  onClick={() => moveTo(uuid, "top")}
                />
                <IconButton
                  icon="arrowUp"
                  size="sm"
                  label={text.moveUp(name)}
                  data-uuid={uuid}
                  data-move="up"
                  disabled={locked || index === 0}
                  onClick={() => moveTo(uuid, "up")}
                />
                <IconButton
                  icon="arrowDown"
                  size="sm"
                  label={text.moveDown(name)}
                  data-uuid={uuid}
                  data-move="down"
                  disabled={locked || index === order.length - 1}
                  onClick={() => moveTo(uuid, "down")}
                />
                <IconButton
                  icon="arrowBottom"
                  size="sm"
                  label={text.moveBottom(name)}
                  data-uuid={uuid}
                  data-move="bottom"
                  disabled={locked || index === order.length - 1}
                  onClick={() => moveTo(uuid, "bottom")}
                />
              </span>
            </li>
          );
        })}
      </ol>
      <div className="fl-device-order__actions">
        <span className="fl-send">
          <Button
            variant="primary"
            icon="check"
            className="fl-send__button"
            aria-disabled={!dirty || locked || undefined}
            aria-describedby={
              busy
                ? "fl-device-order-busy"
                : !dirty
                  ? "fl-device-order-unchanged"
                  : undefined
            }
            onClick={() => void save()}
          >
            {saving ? text.saving : text.save}
          </Button>
          {busy ? (
            <span id="fl-device-order-busy" className="fl-send__reason">
              {text.busy}
            </span>
          ) : (
            !dirty && (
              <span id="fl-device-order-unchanged" className="fl-send__reason">
                {text.unchangedHint}
              </span>
            )
          )}
        </span>
        <Button
          className="fl-send__button"
          aria-disabled={saving || undefined}
          onClick={() => {
            if (!saving) onClose();
          }}
        >
          {text.cancel}
        </Button>
      </div>
    </section>
  );
}
