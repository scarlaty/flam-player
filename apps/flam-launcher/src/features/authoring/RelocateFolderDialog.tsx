import { useContext, useState } from "react";
import { relocateStoryFolder } from "../../api/authoring";
import { pickFolder } from "../../api/dialog";
import { toAppError } from "../../api/error";
import { Banner, Button, Dialog } from "../../components";
import { ToastContext } from "../../components/Toast/ToastContext";
import { t } from "../../i18n";
import { StoriesContext } from "../stories/StoriesContext";
import { errorText } from "./problemText";

export interface RelocateFolderDialogProps {
  /** Identifiant de l'histoire installée dont le dossier est introuvable. */
  uuid: string;
  open: boolean;
  onClose: () => void;
  /** Dossier retenu (forme résolue) : « Mes histoires » est déjà à jour. */
  onRelocated: (folder: string) => void | Promise<void>;
}

/**
 * « Choisir le dossier » d'une histoire de Mes histoires dont le dossier est introuvable (§ 8) :
 * le dossier choisi doit porter le même identifiant (`authoring_relocate`). Partagé par
 * « Actualiser » (`CreateStoryFromFolder`) et « Modifier » (`OpenEditorButton`).
 */
export function RelocateFolderDialog({
  uuid,
  open,
  onClose,
  onRelocated,
}: RelocateFolderDialogProps) {
  const toast = useContext(ToastContext);
  const stories = useContext(StoriesContext);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function relocate() {
    setError(null);
    let kept: string;
    try {
      const picked = await pickFolder(t.authoringFolder.chooseTitle);
      if (picked === null) return;
      setBusy(true);
      try {
        kept = await relocateStoryFolder(uuid, picked);
      } catch (caught) {
        setError(errorText(toAppError(caught)));
        return;
      }
      toast?.show({ message: t.authoringFolder.relocated, tone: "success" });
      // Occupé jusqu'au bout : pas de second « Choisir le dossier » pendant la suite.
      await stories?.refreshInstalled();
      await onRelocated(kept);
    } catch (caught) {
      setError(errorText(toAppError(caught)));
    } finally {
      setBusy(false);
    }
  }

  function close() {
    if (busy) return;
    setError(null);
    onClose();
  }

  return (
    <Dialog
      open={open}
      onClose={close}
      title={t.authoringFolder.missingTitle}
      description={t.authoringFolder.missing}
      footer={
        <>
          <Button variant="ghost" disabled={busy} onClick={close}>
            {t.authoring.cancel}
          </Button>
          <Button
            variant="primary"
            icon="folder"
            disabled={busy}
            aria-busy={busy || undefined}
            onClick={() => void relocate()}
          >
            {t.authoringFolder.choose}
          </Button>
        </>
      }
    >
      {error && (
        <Banner tone="danger" role="alert">
          <p>{error}</p>
        </Banner>
      )}
    </Dialog>
  );
}
