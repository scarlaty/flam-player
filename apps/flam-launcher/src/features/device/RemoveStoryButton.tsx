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
  busy,
  locked = false,
  onWorkingChange,
  onRemoved,
}: RemoveStoryButtonProps) {
  const { removeFromDevice, deviceKey, ejecting } = useDevice();
  const keyAtOpen = useRef<string | null>(null);
  const toast = useToast();
  const [confirming, setConfirming] = useState(false);
  const [working, setWorking] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const hintId = useId();
  const blocked = busy || working || locked || ejecting;
  const name = story.title ?? t.deviceScreen.stories.untitled(story.short_id);

  async function confirmRemove() {
    // Le dialogue se ferme aussitôt : le retrait dure plusieurs secondes, l'état est sur la ligne.
    setConfirming(false);
    setWorking(true);
    onWorkingChange?.(true);
    try {
      const { cleanupPending } = await removeFromDevice(story.uuid, keyAtOpen.current);
      toast.show({
        message: text.done(name) + (cleanupPending ? text.cleanupPending : ""),
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
            setConfirming(true);
          }}
        >
          {working ? text.working : text.button}
        </Button>
        {blocked && !working && (
          <span id={hintId} className="fl-send__reason">
            {busy ? text.busy : ejecting ? text.ejecting : text.otherWorking}
          </span>
        )}
      </span>
      <Dialog
        open={confirming}
        onClose={() => setConfirming(false)}
        role="alertdialog"
        title={text.title(name)}
        description={text.description}
        initialFocus={cancelRef}
        footer={
          <>
            <Button ref={cancelRef} onClick={() => setConfirming(false)}>
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
