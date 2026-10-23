import { useState, type FormEvent } from "react";
import { toAppError, type AppError } from "../../api/error";
import {
  addSource,
  updateSource,
  type Source,
  type SourceInput,
} from "../../api/sources";
import { Button, Dialog, TextField } from "../../components";
import { t } from "../../i18n";
import { ErrorBanner } from "../ErrorBanner";
import { validateCatalogUrl, validateName } from "../validation";

const FORM_ID = "source-form";

interface CatalogDialogProps {
  onClose: () => void;
  onAdded: (source: Source) => void;
}

/** Formulaire d'ajout d'un catalogue : validation en direct, erreurs du backend en bandeau. */
export function AddCatalogDialog({ onClose, onAdded }: CatalogDialogProps) {
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [touched, setTouched] = useState({ name: false, url: false });
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);

  const nameError = touched.name ? validateName(name) : null;
  const urlError = touched.url ? validateCatalogUrl(url) : null;

  async function submit(event: FormEvent) {
    event.preventDefault();
    setTouched({ name: true, url: true });
    if (validateName(name) || validateCatalogUrl(url)) return;
    setBusy(true);
    setError(null);
    try {
      const source = await addSource({
        type: "catalog",
        name: name.trim(),
        url: url.trim(),
      });
      onAdded(source);
    } catch (caught) {
      setError(toAppError(caught));
      setBusy(false);
    }
  }

  const text = t.sourcesScreen.catalogDialog;
  return (
    <Dialog
      open
      onClose={onClose}
      title={text.title}
      description={text.description}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t.common.cancel}
          </Button>
          <Button variant="primary" type="submit" form={FORM_ID} disabled={busy}>
            {text.submit}
          </Button>
        </>
      }
    >
      <form id={FORM_ID} className="fl-form" noValidate onSubmit={submit}>
        {error && <ErrorBanner error={error} />}
        <TextField
          label={text.name}
          value={name}
          required
          error={nameError}
          onChange={(e) => {
            setName(e.target.value);
            setTouched((s) => ({ ...s, name: true }));
          }}
        />
        <TextField
          label={text.url}
          value={url}
          required
          inputMode="url"
          hint={text.urlHint}
          error={urlError}
          onChange={(e) => {
            setUrl(e.target.value);
            setTouched((s) => ({ ...s, url: true }));
          }}
        />
      </form>
    </Dialog>
  );
}

interface RenameDialogProps {
  /** Les flux podcast ne se renomment pas encore (#212). */
  source: Exclude<Source, { type: "library" } | { type: "podcast" }>;
  onClose: () => void;
  onRenamed: (source: Source) => void;
}

/** Renommage d'une source (l'URL ou le chemin sont conservés tels quels). */
export function RenameSourceDialog({ source, onClose, onRenamed }: RenameDialogProps) {
  const [name, setName] = useState(source.name);
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  const [touched, setTouched] = useState(false);
  const nameError = touched ? validateName(name) : null;

  async function submit(event: FormEvent) {
    event.preventDefault();
    setTouched(true);
    if (validateName(name)) return;
    const input: SourceInput =
      source.type === "catalog"
        ? { type: "catalog", name: name.trim(), url: source.url }
        : { type: "folder", name: name.trim(), path: source.path };
    setBusy(true);
    setError(null);
    try {
      onRenamed(await updateSource(source.id, input));
    } catch (caught) {
      setError(toAppError(caught));
      setBusy(false);
    }
  }

  const text = t.sourcesScreen.renameDialog;
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
          <Button variant="primary" type="submit" form={FORM_ID} disabled={busy}>
            {text.submit}
          </Button>
        </>
      }
    >
      <form id={FORM_ID} className="fl-form" noValidate onSubmit={submit}>
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
