import { useRef, useState } from "react";
import { Button, CheckboxField, Dialog } from "../../components";
import { t } from "../../i18n";

export interface RemoveStoryDialogProps {
  open: boolean;
  title: string;
  onCancel: () => void;
  /** Appelé à la confirmation, avec le choix « Supprimer aussi la progression ». */
  onConfirm: (purgeSaves: boolean) => void;
}

/** Confirmation de suppression d'une histoire installée (fiche et liste). */
export function RemoveStoryDialog({
  open,
  title,
  onCancel,
  onConfirm,
}: RemoveStoryDialogProps) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  const [purgeSaves, setPurgeSaves] = useState(false);
  const text = t.stories.removeDialog;
  // Remise à zéro à la fermeture (pas à la confirmation : la fiche reste ouverte pendant l'appel).
  if (!open && purgeSaves) setPurgeSaves(false);
  return (
    <Dialog
      open={open}
      onClose={onCancel}
      role="alertdialog"
      title={text.title(title)}
      description={text.description}
      initialFocus={cancelRef}
      footer={
        <>
          <Button ref={cancelRef} onClick={onCancel}>
            {t.common.cancel}
          </Button>
          <Button variant="danger" onClick={() => onConfirm(purgeSaves)}>
            {text.confirm}
          </Button>
        </>
      }
    >
      <CheckboxField
        label={text.purgeSaves}
        hint={text.purgeSavesHint}
        checked={purgeSaves}
        onChange={(event) => setPurgeSaves(event.target.checked)}
      />
    </Dialog>
  );
}
