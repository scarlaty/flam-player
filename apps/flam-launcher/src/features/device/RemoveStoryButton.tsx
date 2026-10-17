import { useId, useRef, useState } from "react";
import type { DeviceStory } from "../../api/device";
import { toAppError } from "../../api/error";
import { Button, Dialog, useToast } from "../../components";
import { t } from "../../i18n";
import { useDevice } from "./DeviceContext";
import "./SendToDeviceButton.css";

const text = t.deviceTransfer.remove;

export interface RemoveStoryButtonProps {
  story: DeviceStory;
  /** Un envoi est en cours : le retrait est refusé (raison écrite à côté). */
  busy: boolean;
  /** Appelé après un retrait réussi (le bouton disparaît avec sa ligne : le focus doit aller ailleurs). */
  onRemoved: () => void;
}

/** « Retirer » une histoire de la Flam, après confirmation. */
export function RemoveStoryButton({ story, busy, onRemoved }: RemoveStoryButtonProps) {
  const { removeFromDevice } = useDevice();
  const toast = useToast();
  const [confirming, setConfirming] = useState(false);
  const [working, setWorking] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const hintId = useId();
  const name = story.title ?? t.deviceScreen.stories.untitled(story.short_id);

  async function confirmRemove() {
    setWorking(true);
    try {
      const { cleanupPending } = await removeFromDevice(story.uuid);
      setConfirming(false);
      toast.show({
        message: text.done(name) + (cleanupPending ? text.cleanupPending : ""),
        tone: "success",
      });
      onRemoved();
    } catch (caught) {
      setConfirming(false);
      toast.show({ message: text.failed(toAppError(caught).message), tone: "danger" });
    } finally {
      setWorking(false);
    }
  }

  return (
    <>
      <span className="fl-send">
        <Button
          size="sm"
          variant="danger"
          icon="trash"
          className="fl-send__button"
          aria-disabled={busy || undefined}
          aria-describedby={busy ? hintId : undefined}
          aria-label={`${text.button} « ${name} »`}
          onClick={() => {
            if (!busy) setConfirming(true);
          }}
        >
          {text.button}
        </Button>
        {busy && (
          <span id={hintId} className="fl-send__reason">
            {text.busy}
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
