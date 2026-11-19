import { useState } from "react";
import { authoringCode, relocateStoryFolder } from "../../../api/authoring";
import { pickFolder } from "../../../api/dialog";
import {
  closeEditorWindow,
  currentWindowLabel,
  notifyRelocated,
  openEditor,
} from "../../../api/editor";
import { toAppError } from "../../../api/error";
import { Banner, Button } from "../../../components";
import { t } from "../../../i18n";
import { errorText } from "../problemText";
import { openErrorText } from "./errors";

export interface FolderMissingBannerProps {
  /**
   * Identifiant de l'histoire si elle est déjà dans Mes histoires : le dossier choisi y devient
   * son dossier retenu (`authoring_relocate`, il doit porter le même identifiant).
   */
  uuid: string | null;
  /** Échec de l'ouverture du dossier choisi (message déjà rédigé). */
  onError: (message: string) => void;
}

/**
 * Dossier de l'éditeur disparu (lot 9, § 4.5) : l'éditeur est en lecture seule. « Choisir le
 * dossier » ouvre le dossier choisi dans un éditeur (ou ramène le sien) puis ferme celui-ci ;
 * une histoire de Mes histoires y retient son nouveau dossier (§ 8).
 */
export function FolderMissingBanner({ uuid, onError }: FolderMissingBannerProps) {
  const text = t.authoringEditor.watch;
  const [opening, setOpening] = useState(false);

  async function choose() {
    try {
      const path = await pickFolder(text.pickTitle);
      if (path === null) return;
      setOpening(true);
      let folder = path;
      if (uuid !== null) {
        try {
          folder = await relocateStoryFolder(uuid, path);
          // Mes histoires (fenêtre principale) relit son dossier retenu.
          await notifyRelocated();
        } catch (caught) {
          const failure = toAppError(caught);
          // Histoire installée autrement que depuis ce dossier : rien à retenir, on ouvre.
          if (authoringCode(failure) !== "not_from_folder") {
            onError(errorText(failure));
            return;
          }
        }
      }
      const opened = await openEditor(folder);
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
