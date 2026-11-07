import { useRef } from "react";
import type { LibraryFormat } from "../../api/library";
import { Button, Dialog } from "../../components";
import { t } from "../../i18n";

export interface FormatReplaceDialogProps {
  open: boolean;
  /** Titre de l'histoire à installer. */
  story: string;
  /** Format de la version déjà installée (même `uuid`), qui sera remplacée. */
  installed: LibraryFormat;
  /** L'utilisateur confirme le remplacement. */
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * Confirmation avant d'installer une histoire déjà présente dans « Mes histoires » dans l'autre
 * format (histoire Telmi et son pack STUdio de même identifiant, Q10) : la bibliothèque n'en
 * garde qu'une. « Annuler » a le focus.
 */
export function FormatReplaceDialog({
  open,
  story,
  installed,
  onConfirm,
  onCancel,
}: FormatReplaceDialogProps) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  const text = t.stories.formatReplace;
  return (
    <Dialog
      open={open}
      onClose={onCancel}
      role="alertdialog"
      title={text.title(story)}
      description={text.description(installed)}
      initialFocus={cancelRef}
      footer={
        <>
          <Button ref={cancelRef} onClick={onCancel}>
            {text.cancel}
          </Button>
          <Button variant="primary" onClick={onConfirm}>
            {text.confirm}
          </Button>
        </>
      }
    >
      <p>{installed === "flam" ? text.toLunii : text.toFlam}</p>
    </Dialog>
  );
}
