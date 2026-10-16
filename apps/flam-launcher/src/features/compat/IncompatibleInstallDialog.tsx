import { useRef } from "react";
import { Banner, Button, Dialog } from "../../components";
import { t } from "../../i18n";
import { reasonText, type CompatInfo } from "./compatInfo";

/** Vrai si l'installation doit être précédée de l'avertissement (histoire incompatible). */
export function needsInstallWarning(info: CompatInfo | undefined): boolean {
  return info?.level === "incompatible";
}

export interface IncompatibleInstallDialogProps {
  open: boolean;
  /** Titre de l'histoire. */
  story: string;
  info: CompatInfo;
  /** L'utilisateur installe malgré l'avertissement. */
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * Avertissement **non bloquant** avant d'installer une histoire marquée incompatible : la
 * raison est rappelée, « Annuler » a le focus, « Installer quand même » poursuit.
 */
export function IncompatibleInstallDialog({
  open,
  story,
  info,
  onConfirm,
  onCancel,
}: IncompatibleInstallDialogProps) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  return (
    <Dialog
      open={open}
      onClose={onCancel}
      role="alertdialog"
      title={t.compat.install.title(story)}
      description={t.compat.install.description}
      initialFocus={cancelRef}
      footer={
        <>
          <Button ref={cancelRef} onClick={onCancel}>
            {t.compat.install.cancel}
          </Button>
          <Button variant="primary" onClick={onConfirm}>
            {t.compat.install.confirm}
          </Button>
        </>
      }
    >
      <Banner tone="warning">
        <p>{reasonText(info)}</p>
      </Banner>
      <p>{t.compat.install.body}</p>
    </Dialog>
  );
}
