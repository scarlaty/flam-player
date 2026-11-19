import { useContext, useState } from "react";
import { pickFolder } from "../../../api/dialog";
import { openEditor } from "../../../api/editor";
import { toAppError } from "../../../api/error";
import { Banner, Button, type ButtonSize } from "../../../components";
import { ToastContext } from "../../../components/Toast/ToastContext";
import { t } from "../../../i18n";
import { RelocateFolderDialog } from "../RelocateFolderDialog";
import { openErrorText } from "./errors";

export interface OpenEditorButtonProps {
  /** Dossier connu (« Modifier » une histoire de Mes histoires) ; sinon sélecteur natif. */
  folder?: string;
  /**
   * Identifiant de l'histoire installée (avec `folder`) : son dossier introuvable propose
   * « Choisir le dossier » (`authoring_relocate`), puis l'éditeur s'ouvre sur le dossier choisi.
   */
  uuid?: string;
  size?: ButtonSize;
}

/**
 * Ouvre un dossier dans la fenêtre de l'éditeur : « Créer une histoire depuis un dossier »
 * (sélecteur de dossier) ou « Modifier » (dossier source d'une histoire). Un dossier déjà ouvert
 * ramène sa fenêtre au premier plan, avec un message.
 */
export function OpenEditorButton({ folder, uuid, size = "md" }: OpenEditorButtonProps) {
  const text = t.authoringEditor;
  const toast = useContext(ToastContext);
  const [opening, setOpening] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Dossier de l'histoire introuvable : « Choisir le dossier » (§ 8).
  const [missing, setMissing] = useState(false);

  async function open(known: string | null = folder ?? null) {
    setError(null);
    try {
      const path = known ?? (await pickFolder(text.pickTitle));
      if (path === null) return;
      setOpening(true);
      const opened = await openEditor(path);
      if (opened.already_open) toast?.show({ message: text.alreadyOpen, tone: "info" });
    } catch (caught) {
      const failure = toAppError(caught);
      if (uuid !== undefined && known === folder && failure.code === "folder_missing") {
        setMissing(true);
        return;
      }
      const message = openErrorText(failure);
      if (toast) toast.show({ message, tone: "danger" });
      else setError(message);
    } finally {
      setOpening(false);
    }
  }

  return (
    <>
      <Button
        size={size}
        variant={folder ? "secondary" : "accent"}
        icon={folder ? "pencil" : "plus"}
        title={folder ? text.editHelp : undefined}
        disabled={opening}
        aria-busy={opening || undefined}
        onClick={() => void open()}
      >
        {opening ? text.opening : folder ? text.edit : text.open}
      </Button>
      {uuid !== undefined && (
        <RelocateFolderDialog
          uuid={uuid}
          open={missing}
          onClose={() => setMissing(false)}
          onRelocated={async (kept) => {
            setMissing(false);
            await open(kept);
          }}
        />
      )}
      {error && (
        <Banner tone="danger" role="alert">
          <p>{error}</p>
        </Banner>
      )}
    </>
  );
}
