import { useId, useRef, useState } from "react";
import { Button, CheckboxField, Dialog } from "../../components";
import { t } from "../../i18n";

export interface RemoveStoryDialogProps {
  open: boolean;
  title: string;
  /**
   * Suppression en cours : « Supprimer » est refusé (pas de second appel), « Annuler » devient
   * « Fermer » et « Suppression en cours. » est annoncé. La fermeture reste possible (Échap,
   * fond, Fermer) : si l'appel ne répond jamais, le dialogue n'est pas piégé. Fermer n'annule
   * pas la suppression, qui continue (progression comprise si la case est cochée).
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
  const statusId = useId();
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
            {busy ? t.common.close : t.common.cancel}
          </Button>
          <Button
            variant="danger"
            aria-disabled={busy || undefined}
            aria-busy={busy || undefined}
            aria-describedby={busy ? statusId : undefined}
            onClick={() => {
              if (!busy) onConfirm(purgeSaves);
            }}
          >
            {text.confirm}
          </Button>
        </>
      }
    >
      <p id={statusId} role="status" className="fl-visually-hidden">
        {busy ? t.stories.actions.removing : ""}
      </p>
      <CheckboxField
        label={text.purgeSaves}
        hint={text.purgeSavesHint}
        checked={purgeSaves}
        onChange={(event) => setPurgeSaves(event.target.checked)}
      />
    </Dialog>
  );
}
