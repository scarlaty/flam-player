import { useContext, useState } from "react";
import {
  authoringCode,
  createStoryFromFolder,
  readStoryFolder,
  relocateStoryFolder,
  type ConflictChoice,
  type FolderReport,
} from "../../api/authoring";
import { pickFolder } from "../../api/dialog";
import { toAppError } from "../../api/error";
import { Banner, Button, Dialog, RadioGroup, type ButtonSize } from "../../components";
import { ToastContext } from "../../components/Toast/ToastContext";
import { t } from "../../i18n";
import { StoriesContext } from "../stories/StoriesContext";
import { errorText, problemText } from "./problemText";
import "./CreateStoryFromFolder.css";

/** Lignes montrées au plus par liste (le reste est résumé en « Et N autres choses »). */
const MAX_LINES = 5;

export interface CreateStoryFromFolderProps {
  /** Dossier déjà connu (« Mettre à jour » une histoire de Mes histoires) ; sinon sélecteur. */
  folder?: string;
  /**
   * Identifiant de l'histoire installée (avec `folder`) : son dossier introuvable propose
   * « Choisir le dossier » (le dossier choisi doit porter le même identifiant).
   */
  uuid?: string;
  size?: ButtonSize;
}

function Lines({ items }: { items: string[] }) {
  const shown = items.slice(0, MAX_LINES);
  return (
    <ul className="fl-authoring__lines">
      {shown.map((line, index) => (
        <li key={index}>{line}</li>
      ))}
      {items.length > shown.length && (
        <li>{t.authoring.more(items.length - shown.length)}</li>
      )}
    </ul>
  );
}

/**
 * « Créer une histoire depuis un dossier » (ou « Actualiser » avec `folder`) : choisit le dossier,
 * montre ce qu'il contient (nombre d'histoires, points à corriger, avertissements), fait trancher
 * un dossier copié, puis lance la création (tâche de la file d'installation). Pas d'éditeur ici
 * (lots 7 et 8 de docs/creation-dossier.md).
 */
export function CreateStoryFromFolder({
  folder,
  uuid,
  size = "md",
}: CreateStoryFromFolderProps) {
  const text = t.authoring;
  const toast = useContext(ToastContext);
  const stories = useContext(StoriesContext);
  const [reading, setReading] = useState(false);
  const [report, setReport] = useState<FolderReport | null>(null);
  const [choice, setChoice] = useState<ConflictChoice>("new_story");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Dossier de l'histoire introuvable : « Choisir le dossier » (§ 8).
  const [missing, setMissing] = useState(false);
  const [relocateError, setRelocateError] = useState<string | null>(null);

  async function open(known: string | null = folder ?? null) {
    setError(null);
    let path = known;
    try {
      if (path === null) path = await pickFolder(text.pickTitle);
      if (path === null) return;
      setReading(true);
      const read = await readStoryFolder(path);
      setChoice("new_story");
      setReport(read);
    } catch (caught) {
      const failure = toAppError(caught);
      if (uuid !== undefined && authoringCode(failure) === "folder_missing") {
        setRelocateError(null);
        setMissing(true);
        return;
      }
      const message = errorText(failure);
      if (toast) toast.show({ message, tone: "danger" });
      else setError(message);
    } finally {
      setReading(false);
    }
  }

  async function relocate() {
    if (uuid === undefined) return;
    setRelocateError(null);
    let kept: string;
    try {
      const picked = await pickFolder(t.authoringFolder.chooseTitle);
      if (picked === null) return;
      setBusy(true);
      kept = await relocateStoryFolder(uuid, picked);
    } catch (caught) {
      setRelocateError(errorText(toAppError(caught)));
      return;
    } finally {
      setBusy(false);
    }
    setMissing(false);
    toast?.show({ message: t.authoringFolder.relocated, tone: "success" });
    await stories?.refreshInstalled();
    await open(kept);
  }

  async function create() {
    if (!report) return;
    setBusy(true);
    setError(null);
    try {
      const conflict = report.conflict;
      const answer: ConflictChoice | null =
        conflict === null ? null : conflict.kind === "foreign" ? "new_story" : choice;
      const created = await createStoryFromFolder(report.folder, answer);
      stories?.trackTask(created.task);
      toast?.show({ message: text.started, tone: "success" });
      setReport(null);
    } catch (caught) {
      setError(errorText(toAppError(caught)));
    } finally {
      setBusy(false);
    }
  }

  const blocking = report?.problems.filter((p) => p.severity === "blocking") ?? [];
  const warnings = report?.problems.filter((p) => p.severity === "warning") ?? [];
  const updating =
    report !== null && report.uuid !== null && report.installed_version !== null;
  const conflict = report?.conflict ?? null;

  return (
    <>
      <Button
        size={size}
        variant={folder ? "secondary" : "accent"}
        icon={folder ? "refresh" : "plus"}
        disabled={reading}
        aria-busy={reading || undefined}
        onClick={() => void open()}
      >
        {reading ? text.reading : folder ? text.refresh : text.open}
      </Button>
      {error && report === null && (
        <Banner tone="danger" role="alert">
          <p>{error}</p>
        </Banner>
      )}
      <Dialog
        open={missing}
        onClose={() => {
          if (!busy) setMissing(false);
        }}
        title={t.authoringFolder.missingTitle}
        description={t.authoringFolder.missing}
        footer={
          <>
            <Button variant="ghost" disabled={busy} onClick={() => setMissing(false)}>
              {text.cancel}
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
        {relocateError && (
          <Banner tone="danger" role="alert">
            <p>{relocateError}</p>
          </Banner>
        )}
      </Dialog>
      <Dialog
        open={report !== null}
        onClose={() => {
          if (!busy) setReport(null);
        }}
        title={report?.title ?? ""}
        description={text.dialogDescription}
        footer={
          report?.blocked ? (
            <Button onClick={() => setReport(null)}>{text.close}</Button>
          ) : (
            <>
              <Button variant="ghost" disabled={busy} onClick={() => setReport(null)}>
                {text.cancel}
              </Button>
              <Button
                variant="primary"
                icon="sparkle"
                disabled={busy}
                aria-busy={busy || undefined}
                onClick={() => void create()}
              >
                {busy ? text.creating : updating ? text.update : text.create}
              </Button>
            </>
          )
        }
      >
        {report && (
          <div className="fl-authoring">
            <p className="fl-authoring__count">
              {text.count(report.story_count, report.mode === "chapitres")}
            </p>
            {blocking.length > 0 && (
              <Banner tone="warning" title={text.fixFirst} role="none">
                <Lines items={blocking.map(problemText)} />
              </Banner>
            )}
            {blocking.length === 0 && warnings.length > 0 && (
              <Banner tone="info" title={text.goodToKnow} role="none">
                <Lines items={[...new Set(warnings.map(problemText))]} />
              </Banner>
            )}
            {!report.blocked && conflict?.kind === "copied" && (
              <RadioGroup
                legend={text.copied.legend}
                value={choice}
                onChange={setChoice}
                disabled={busy}
                options={[
                  { value: "new_story", label: text.copied.newStory },
                  { value: "replace_other", label: text.copied.replace(conflict.title) },
                ]}
              />
            )}
            {!report.blocked && conflict?.kind === "foreign" && <p>{text.foreign}</p>}
            {error && (
              <Banner tone="danger" role="alert">
                <p>{error}</p>
              </Banner>
            )}
          </div>
        )}
      </Dialog>
    </>
  );
}
