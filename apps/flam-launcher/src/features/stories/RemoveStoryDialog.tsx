import { useRef, useState } from "react";
import { Button, CheckboxField, Dialog } from "../../components";
import { t } from "../../i18n";

export interface RemoveStoryDialogProps {
  open: boolean;
  title: string;
  /**
   * Suppression en cours : « Supprimer » est refusé (pas de second appel) et la fermeture
   * (Annuler, Échap, fond, Fermer) est ignorée : la suppression continue, le dialogue reste.
   */
  busy?: boolean;
  onCancel: () => void;
  /** Appelé à la confirmation, avec le choix « Supprimer aussi la progression ». */
  onConfirm: (purgeSaves: boolean) => void;
}

/** Confirmation de suppression d'une histoire installée (fiche et liste). */
export function RemoveStoryDialog({
  open,
  title,
  busy = false,
  onCancel,
  onConfirm,
}: RemoveStoryDialogProps) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  const [purgeSaves, setPurgeSaves] = useState(false);
  const text = t.stories.removeDialog;
  // Remise à zéro à la fermeture (pas à la confirmation : la fiche reste ouverte pendant l'appel).
  if (!open && purgeSaves) setPurgeSaves(false);
  const cancel = () => {
    if (!busy) onCancel();
  };
  return (
    <Dialog
      open={open}
      onClose={cancel}
      role="alertdialog"
      title={text.title(title)}
      description={text.description}
      initialFocus={cancelRef}
      footer={
        <>
          <Button ref={cancelRef} aria-disabled={busy || undefined} onClick={cancel}>
            {t.common.cancel}
          </Button>
          <Button
            variant="danger"
            aria-disabled={busy || undefined}
            aria-busy={busy || undefined}
            onClick={() => {
              if (!busy) onConfirm(purgeSaves);
            }}
          >
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
