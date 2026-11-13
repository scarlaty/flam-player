import { useId, useRef, useState } from "react";
import type { DeviceStory } from "../../api/device";
import { toAppError } from "../../api/error";
import { Button, Dialog, useToast } from "../../components";
import { cx } from "../../components/cx";
import { t } from "../../i18n";
import { useDevice } from "./DeviceContext";
import "./SendToDeviceButton.css";

const text = t.deviceTransfer.remove;

export interface RemoveStoryButtonProps {
  story: DeviceStory;
  /**
   * Nom affiché par la ligne de l'histoire (titre déchiffré de la Flam, sinon catalogue
   * officiel, sinon bibliothèque, sinon « Histoire xxxxxxxx ») : bouton, confirmation et
   * notification le reprennent.
   */
  name: string;
  /** Un envoi est en cours : le retrait est refusé (raison écrite à côté). */
  busy: boolean;
  /** Un autre retrait est en cours : celui-ci est refusé (raison écrite à côté). */
  locked?: boolean;
  /** Prévient l'écran quand le retrait démarre ou se termine (la ligne est alors grisée). */
  onWorkingChange?: (working: boolean) => void;
  /** Appelé après un retrait réussi (le bouton disparaît avec sa ligne : le focus doit aller ailleurs). */
  onRemoved: () => void;
}

/** « Retirer » une histoire de la Flam, après confirmation. */
export function RemoveStoryButton({
  story,
  name,
  busy,
  locked = false,
  onWorkingChange,
  onRemoved,
}: RemoveStoryButtonProps) {
  const { removeFromDevice, deviceKey, ejecting, canMaintain } = useDevice();
  const keyAtOpen = useRef<string | null>(null);
  const toast = useToast();
  // Nom figé à l'ouverture : les titres de la Flam ou du catalogue peuvent arriver pendant que
  // le dialogue est ouvert, la question et la notification gardent celui que l'utilisateur a lu.
  const [confirming, setConfirming] = useState<string | null>(null);
  const [working, setWorking] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const hintId = useId();
  // Garde du firmware (#326) : Flam 2.x non confirmée ou écriture bloquée.
  const firmware = !canMaintain;
  const blocked = busy || working || locked || ejecting || firmware;

  async function confirmRemove() {
    // Le dialogue se ferme aussitôt : le retrait dure plusieurs secondes, l'état est sur la ligne.
    const confirmedName = confirming ?? name;
    setConfirming(null);
    setWorking(true);
    onWorkingChange?.(true);
    try {
      const { cleanupPending } = await removeFromDevice(story.uuid, keyAtOpen.current);
      toast.show({
        message: text.done(confirmedName) + (cleanupPending ? text.cleanupPending : ""),
        tone: "success",
      });
      onRemoved();
    } catch (caught) {
      toast.show({ message: text.failed(toAppError(caught).message), tone: "danger" });
    } finally {
      setWorking(false);
      onWorkingChange?.(false);
    }
  }

  return (
    <>
      <span className="fl-send">
        <Button
          size="sm"
          variant="danger"
          icon={working ? "refresh" : "trash"}
          className={cx("fl-send__button", working && "fl-send__button--working")}
          aria-disabled={blocked || undefined}
          aria-busy={working || undefined}
          aria-describedby={blocked && !working ? hintId : undefined}
          aria-label={`${working ? text.working : text.button} « ${name} »`}
          onClick={() => {
            if (blocked) return;
            keyAtOpen.current = deviceKey;
            setConfirming(name);
          }}
        >
          {working ? text.working : text.button}
        </Button>
        {blocked && !working && (
          // « Un autre retrait est en cours » et le blocage du firmware : lus par les lecteurs
          // d'écran seulement, pour ne pas répéter la même phrase sur chaque ligne (la ligne
          // concernée montre « Retrait… », le bandeau du firmware explique le blocage).
          <span
            id={hintId}
            className={
              blocked && !busy && !ejecting ? "fl-visually-hidden" : "fl-send__reason"
            }
          >
            {busy
              ? text.busy
              : ejecting
                ? text.ejecting
                : firmware
                  ? text.firmware
                  : text.otherWorking}
          </span>
        )}
      </span>
      <Dialog
        open={confirming !== null}
        onClose={() => setConfirming(null)}
        role="alertdialog"
        title={text.title(confirming ?? name)}
        description={text.description}
        initialFocus={cancelRef}
        footer={
          <>
            <Button ref={cancelRef} onClick={() => setConfirming(null)}>
              {t.common.cancel}
            </Button>
            <Button
              variant="danger"
              aria-disabled={working || undefined}
              onClick={() => {
                if (!working) void confirmRemove();
              }}
            >
              {text.confirm}
            </Button>
          </>
        }
      />
    </>
  );
}
