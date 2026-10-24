import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { toAppError, type AppError } from "../../api/error";
import {
  deletePackComposition,
  listPackCompositions,
  openPackComposition,
  readPackDraft,
  renamePackComposition,
  type CompositionSummary,
  type CompositionsView,
  type PackComposition,
} from "../../api/pack";
import {
  Banner,
  Button,
  Dialog,
  IconButton,
  TextField,
  useToast,
} from "../../components";
import { t } from "../../i18n";
import { PACK_HASH } from "../../router";
import { ErrorBanner } from "../ErrorBanner";
import { validateName } from "../validation";
import { settlePendingSave } from "./pack/hooks";

/** Longueur d'un nom de composition (miroir de `MAX_NAME_CHARS`, `compositions.rs`). */
export const MAX_COMPOSITION_NAME_CHARS = 60;

const RENAME_FORM_ID = "fl-composition-rename";

function formatDate(seconds: number): string | null {
  if (seconds <= 0) return null;
  return new Date(seconds * 1000).toLocaleDateString("fr-FR", { dateStyle: "long" });
}

/** Affiche l'éditeur de pack, qui relit le brouillon. */
function showEditor() {
  window.location.hash = PACK_HASH;
}

/**
 * Brouillon sans aucune saisie : ni épisode ni catégorie, ni texte, âge ou couverture choisis
 * (champ ajouté plus tard : vide seulement s'il est absent ou `null`). Seul un brouillon vide
 * est remplacé sans confirmation (#484).
 */
export function isBlankComposition(composition: PackComposition): boolean {
  const { metadata, cover, menu, ...rest } = composition;
  const { title, subtitle, description, age, ...metadataRest } = metadata;
  const unset = (value: unknown) => value === null || value === undefined;
  return (
    menu.length === 0 &&
    title.trim() === "" &&
    (subtitle ?? "").trim() === "" &&
    (description ?? "").trim() === "" &&
    age === 0 &&
    cover.kind === "default" &&
    Object.values(rest).every(unset) &&
    Object.values(metadataRest).every(unset)
  );
}

/** Saisie en caractères (points de code), comme `MAX_NAME_CHARS` côté backend. */
function validateCompositionName(raw: string): string | null {
  const error = validateName(raw);
  if (error) return error;
  return [...raw.trim()].length > MAX_COMPOSITION_NAME_CHARS
    ? t.podcasts.compositions.nameTooLong(MAX_COMPOSITION_NAME_CHARS)
    : null;
}

/**
 * Compositions nommées réutilisables (#461, spec § 14) : liste, ouverture dans l'éditeur (le
 * brouillon est remplacé, après confirmation s'il contient une saisie), renommage,
 * suppression.
 */
export function CompositionsSection() {
  const text = t.podcasts.compositions;
  const toast = useToast();
  const [view, setView] = useState<CompositionsView | null>(null);
  const [loadError, setLoadError] = useState<AppError | null>(null);
  const [actionError, setActionError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  /** Composition en cours d'ouverture (indicateur). */
  const [opening, setOpening] = useState<CompositionSummary | null>(null);
  const [replacing, setReplacing] = useState<CompositionSummary | null>(null);
  const [renaming, setRenaming] = useState<CompositionSummary | null>(null);
  const [deleting, setDeleting] = useState<CompositionSummary | null>(null);
  const cancelReplaceRef = useRef<HTMLButtonElement>(null);
  const cancelDeleteRef = useRef<HTMLButtonElement>(null);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const load = useCallback(async () => {
    setLoadError(null);
    try {
      const next = await listPackCompositions();
      if (mounted.current) setView(next);
    } catch (caught) {
      if (mounted.current) setLoadError(toAppError(caught));
    }
  }, []);

  useEffect(() => {
    // Chargement initial depuis le backend.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void load();
  }, [load]);

  /** Erreur d'action : affichée, et la liste relue (composition disparue entre-temps). */
  function fail(caught: unknown) {
    setActionError(toAppError(caught));
    void load();
  }

  /**
   * Message dit une fois par le backend (fichier mis de côté, entrées écartées) : gardé
   * jusqu'à fermeture, il survit au passage à l'éditeur.
   */
  function showNotice(notice: string | null) {
    if (notice) toast.show({ message: notice, tone: "info", duration: 0 });
  }

  async function open(target: CompositionSummary) {
    setReplacing(null);
    setActionError(null);
    setBusy(true);
    setOpening(target);
    try {
      const opened = await openPackComposition(target.id);
      showNotice(opened.notice);
      showEditor();
    } catch (caught) {
      fail(caught);
    } finally {
      if (mounted.current) {
        setBusy(false);
        setOpening(null);
      }
    }
  }

  /** Ouvrir : confirmation dès que le brouillon contient une saisie. */
  async function requestOpen(target: CompositionSummary) {
    setActionError(null);
    setBusy(true);
    setOpening(target);
    let replace: boolean;
    try {
      // Un éditeur fermé à l'instant a pu laisser une sauvegarde en cours.
      await settlePendingSave();
      const draft = await readPackDraft();
      // Brouillon illisible : déjà mis de côté par le backend (copie), son message est dit.
      showNotice(draft.notice);
      replace = draft.composition !== null && !isBlankComposition(draft.composition);
    } catch {
      // Lecture impossible : la confirmation protège ce qui pourrait s'y trouver.
      replace = true;
    } finally {
      if (mounted.current) {
        setBusy(false);
        setOpening(null);
      }
    }
    if (!mounted.current) return;
    if (replace) setReplacing(target);
    else await open(target);
  }

  async function confirmDelete() {
    const target = deleting;
    if (!target) return;
    setDeleting(null);
    setActionError(null);
    setBusy(true);
    try {
      const done = await deletePackComposition(target.id);
      toast.show({ message: text.removed(target.name), tone: "success" });
      showNotice(done.notice);
      await load();
    } catch (caught) {
      fail(caught);
    } finally {
      if (mounted.current) setBusy(false);
    }
  }

  const compositions = view?.compositions ?? [];
  return (
    <section className="fl-podcasts__section" aria-labelledby="fl-podcasts-compositions">
      <h2 id="fl-podcasts-compositions" className="fl-podcasts__heading">
        {text.title}
      </h2>
      <p className="fl-podcasts__hint">{text.intro}</p>
      {view === null && loadError === null && <p role="status">{text.loading}</p>}
      {loadError && (
        <ErrorBanner
          error={{
            kind: loadError.kind,
            message: `${text.loadFailed} ${loadError.message}`,
          }}
          action={
            <Button variant="secondary" size="sm" onClick={() => void load()}>
              {t.errors.retry}
            </Button>
          }
        />
      )}
      {view?.notice && (
        <Banner tone="warning">
          <p>{view.notice}</p>
        </Banner>
      )}
      {actionError && <ErrorBanner error={actionError} />}
      {opening && <p role="status">{text.opening(opening.name)}</p>}
      {view !== null && compositions.length === 0 && <p>{text.empty}</p>}
      {compositions.length > 0 && (
        <>
          <p className="fl-podcasts__hint">
            {text.count(compositions.length, view!.max)}
          </p>
          <ul className="fl-podcasts__compositions">
            {compositions.map((c) => {
              const date = formatDate(c.last_used_at);
              return (
                <li key={c.id} className="fl-podcasts__composition">
                  <div className="fl-podcasts__composition-text">
                    <strong>{c.name}</strong>
                    <span>
                      {text.episodes(c.episodes)}
                      {date && ` · ${text.lastUsed(date)}`}
                    </span>
                    {c.missing_feeds > 0 && (
                      <span className="fl-podcasts__composition-warning">
                        {text.missingFeeds(c.missing_feeds)}
                      </span>
                    )}
                  </div>
                  <div className="fl-podcasts__composition-actions">
                    <Button
                      variant="secondary"
                      size="sm"
                      aria-label={text.openLabel(c.name)}
                      disabled={busy}
                      onClick={() => void requestOpen(c)}
                    >
                      {text.open}
                    </Button>
                    <IconButton
                      icon="pencil"
                      size="sm"
                      label={text.rename(c.name)}
                      disabled={busy}
                      onClick={() => setRenaming(c)}
                    />
                    <IconButton
                      icon="trash"
                      size="sm"
                      label={text.remove(c.name)}
                      disabled={busy}
                      onClick={() => setDeleting(c)}
                    />
                  </div>
                </li>
              );
            })}
          </ul>
        </>
      )}
      {renaming && (
        <RenameCompositionDialog
          composition={renaming}
          onClose={() => setRenaming(null)}
          onRenamed={(name, notice) => {
            setRenaming(null);
            toast.show({ message: text.renamed(name), tone: "success" });
            showNotice(notice);
            void load();
          }}
          onMissing={(caught) => {
            setRenaming(null);
            fail(caught);
          }}
        />
      )}
      <Dialog
        open={replacing !== null}
        onClose={() => setReplacing(null)}
        role="alertdialog"
        title={text.replaceDialog.title}
        description={replacing ? text.replaceDialog.description(replacing.name) : ""}
        initialFocus={cancelReplaceRef}
        footer={
          <>
            <Button
              ref={cancelReplaceRef}
              variant="ghost"
              onClick={() => setReplacing(null)}
            >
              {t.common.cancel}
            </Button>
            <Button
              variant="primary"
              onClick={() => {
                if (replacing) void open(replacing);
              }}
            >
              {text.replaceDialog.confirm}
            </Button>
          </>
        }
      />
      <Dialog
        open={deleting !== null}
        onClose={() => setDeleting(null)}
        role="alertdialog"
        title={text.deleteDialog.title}
        description={deleting ? text.deleteDialog.description(deleting.name) : ""}
        initialFocus={cancelDeleteRef}
        footer={
          <>
            <Button
              ref={cancelDeleteRef}
              variant="ghost"
              onClick={() => setDeleting(null)}
            >
              {t.common.cancel}
            </Button>
            <Button variant="danger" onClick={() => void confirmDelete()}>
              {text.deleteDialog.confirm}
            </Button>
          </>
        }
      />
    </section>
  );
}

interface RenameDialogProps {
  composition: CompositionSummary;
  onClose: () => void;
  onRenamed: (name: string, notice: string | null) => void;
  /** Composition disparue entre-temps : le dialogue se ferme et la liste est relue. */
  onMissing: (error: unknown) => void;
}

function RenameCompositionDialog({
  composition,
  onClose,
  onRenamed,
  onMissing,
}: RenameDialogProps) {
  const text = t.podcasts.compositions.renameDialog;
  const [name, setName] = useState(composition.name);
  const [touched, setTouched] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  const nameError = touched ? validateCompositionName(name) : null;

  async function submit(event: FormEvent) {
    event.preventDefault();
    setTouched(true);
    if (validateCompositionName(name)) return;
    setBusy(true);
    setError(null);
    try {
      const done = await renamePackComposition(composition.id, name.trim());
      onRenamed(name.trim(), done.notice);
    } catch (caught) {
      const appError = toAppError(caught);
      if (appError.kind === "not_found") {
        onMissing(appError);
        return;
      }
      setError(appError);
      setBusy(false);
    }
  }

  return (
    <Dialog
      open
      onClose={onClose}
      title={text.title}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t.common.cancel}
          </Button>
          <Button variant="primary" type="submit" form={RENAME_FORM_ID} disabled={busy}>
            {text.submit}
          </Button>
        </>
      }
    >
      <form id={RENAME_FORM_ID} className="fl-form" noValidate onSubmit={submit}>
        {error && <ErrorBanner error={error} />}
        <TextField
          label={text.name}
          value={name}
          required
          error={nameError}
          onChange={(e) => {
            setName(e.target.value);
            setTouched(true);
          }}
        />
      </form>
    </Dialog>
  );
}
