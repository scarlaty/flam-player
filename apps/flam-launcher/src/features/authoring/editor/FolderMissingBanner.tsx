import { useState } from "react";
import { pickFolder } from "../../../api/dialog";
import { closeEditorWindow, currentWindowLabel, openEditor } from "../../../api/editor";
import { toAppError } from "../../../api/error";
import { Banner, Button } from "../../../components";
import { t } from "../../../i18n";
import { openErrorText } from "./errors";

export interface FolderMissingBannerProps {
  /** Échec de l'ouverture du dossier choisi (message déjà rédigé). */
  onError: (message: string) => void;
}

/**
 * Dossier de l'éditeur disparu (lot 9, § 4.5) : l'éditeur est en lecture seule. « Choisir le
 * dossier » ouvre le dossier choisi dans un éditeur (ou ramène le sien) puis ferme celui-ci.
 */
export function FolderMissingBanner({ onError }: FolderMissingBannerProps) {
  const text = t.authoringEditor.watch;
  const [opening, setOpening] = useState(false);

  async function choose() {
    try {
      const path = await pickFolder(text.pickTitle);
      if (path === null) return;
      setOpening(true);
      const opened = await openEditor(path);
      // Le même dossier revenu entre-temps : c'est cette fenêtre, elle reste ouverte.
      if (opened.label !== currentWindowLabel()) await closeEditorWindow();
    } catch (caught) {
      onError(openErrorText(toAppError(caught)));
    } finally {
      setOpening(false);
    }
  }

  return (
    <Banner
      tone="warning"
      title={text.missingTitle}
      // Déjà annoncé par la région d'état de l'éditeur.
      role="none"
      className="fl-editor__error"
      action={
        <Button
          icon="folder"
          disabled={opening}
          aria-busy={opening || undefined}
          onClick={() => void choose()}
        >
          {text.choose}
        </Button>
      }
    >
      <p>{text.missingHelp}</p>
    </Banner>
  );
}
