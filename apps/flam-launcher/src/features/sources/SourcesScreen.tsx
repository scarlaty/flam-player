import { useCallback, useEffect, useRef, useState } from "react";
import { toAppError, type AppError } from "../../api/error";
import { pickFolder } from "../../api/dialog";
import {
  addSource,
  listSources,
  removeSource,
  reorderSources,
  type Source,
} from "../../api/sources";
import { Badge, Button, Dialog, Icon, IconButton, useToast } from "../../components";
import { t } from "../../i18n";
import { ErrorBanner } from "../ErrorBanner";
import { useRecovery } from "../recovery/RecoveryContext";
import { MAX_NAME_LEN } from "../validation";
import { AddCatalogDialog, RenameSourceDialog } from "./SourceDialogs";
import "./SourcesScreen.css";

type Editable = Exclude<Source, { type: "library" }>;

const isEditable = (source: Source): source is Editable => source.type !== "library";

/** Nom proposé pour un dossier : son dernier segment, borné à la longueur autorisée. */
export function folderDisplayName(path: string): string {
  const last = path.split(/[\\/]/).filter(Boolean).pop() ?? "";
  const name = [...last].slice(0, MAX_NAME_LEN).join("").trim();
  return name || t.sourcesScreen.defaultFolderName;
}

/** Déplace l'élément `from` à la position `to` (copie). */
export function moveItem<T>(list: readonly T[], from: number, to: number): T[] {
  const next = [...list];
  const [item] = next.splice(from, 1);
  if (item === undefined) return next;
  next.splice(to, 0, item);
  return next;
}

/** Écran de gestion des sources : ajout, renommage, ordre (glisser ou clavier), suppression. */
export function SourcesScreen() {
  const toast = useToast();
  const { report } = useRecovery();
  const [sources, setSources] = useState<Source[] | null>(null);
  const [loadError, setLoadError] = useState<AppError | null>(null);
  const [actionError, setActionError] = useState<AppError | null>(null);
  const [announcement, setAnnouncement] = useState("");
  const [adding, setAdding] = useState(false);
  const [renaming, setRenaming] = useState<Editable | null>(null);
  const [deleting, setDeleting] = useState<Editable | null>(null);
  const [busy, setBusy] = useState(false);
  const [dragId, setDragId] = useState<string | null>(null);
  const cancelDeleteRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const pendingFocus = useRef<{ id: string; dir: "up" | "down" } | null>(null);

  const load = useCallback(async () => {
    setLoadError(null);
    try {
      const view = await listSources();
      setSources(view.sources);
      report("sources.json", view.recovery);
    } catch (caught) {
      setLoadError(toAppError(caught));
    }
  }, [report]);

  useEffect(() => {
    // Chargement initial depuis le backend.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void load();
  }, [load]);

  // Redonne le focus au bouton de déplacement après un réordonnancement au clavier.
  useEffect(() => {
    const pending = pendingFocus.current;
    if (!pending || !listRef.current) return;
    pendingFocus.current = null;
    const find = (dir: string) =>
      listRef.current?.querySelector<HTMLButtonElement>(
        `[data-source-id="${pending.id}"][data-move="${dir}"]`,
      );
    const preferred = find(pending.dir);
    const target =
      preferred && !preferred.disabled
        ? preferred
        : find(pending.dir === "up" ? "down" : "up");
    target?.focus();
  }, [sources]);

  const registered = (sources ?? []).filter(isEditable);

  async function persistOrder(next: Editable[], movedId: string) {
    if (!sources) return;
    const previous = sources;
    const library = sources.filter((s) => !isEditable(s));
    setSources([...library, ...next]);
    setBusy(true);
    setActionError(null);
    try {
      const result = await reorderSources(next.map((s) => s.id));
      setSources(result);
      const position = result.filter(isEditable).findIndex((s) => s.id === movedId) + 1;
      const moved = result.find((s) => s.id === movedId);
      if (moved) {
        setAnnouncement(
          t.sourcesScreen.moved(moved.name, position, result.filter(isEditable).length),
        );
      }
    } catch (caught) {
      setSources(previous);
      setActionError(toAppError(caught));
    } finally {
      setBusy(false);
    }
  }

  function moveBy(id: string, delta: -1 | 1) {
    if (busy) return;
    const from = registered.findIndex((s) => s.id === id);
    const to = from + delta;
    if (from < 0 || to < 0 || to >= registered.length) return;
    pendingFocus.current = { id, dir: delta < 0 ? "up" : "down" };
    void persistOrder(moveItem(registered, from, to), id);
  }

  function dropOn(targetId: string) {
    const id = dragId;
    setDragId(null);
    if (!id || id === targetId || busy) return;
    const from = registered.findIndex((s) => s.id === id);
    const to = registered.findIndex((s) => s.id === targetId);
    if (from < 0 || to < 0) return;
    void persistOrder(moveItem(registered, from, to), id);
  }

  async function addFolder() {
    setActionError(null);
    try {
      const path = await pickFolder(t.sourcesScreen.pickFolderTitle);
      if (!path) return;
      const source = await addSource({
        type: "folder",
        name: folderDisplayName(path),
        path,
      });
      onAdded(source);
    } catch (caught) {
      setActionError(toAppError(caught));
    }
  }

  function onAdded(source: Source) {
    setAdding(false);
    setSources((list) => (list ? [...list, source] : list));
    toast.show({ message: t.sourcesScreen.added(source.name), tone: "success" });
  }

  async function confirmDelete() {
    const target = deleting;
    if (!target) return;
    setDeleting(null);
    setActionError(null);
    try {
      await removeSource(target.id);
      setSources((list) => list?.filter((s) => s.id !== target.id) ?? null);
      toast.show({ message: t.sourcesScreen.removed(target.name), tone: "success" });
    } catch (caught) {
      setActionError(toAppError(caught));
    }
  }

  if (loadError) {
    return (
      <ErrorBanner
        error={loadError}
        action={<Button onClick={() => void load()}>{t.errors.retry}</Button>}
      />
    );
  }
  if (!sources) return <p>{t.common.loading}</p>;

  return (
    <div className="fl-sources-screen">
      <p className="fl-sources-screen__intro">{t.sourcesScreen.intro}</p>
      <div className="fl-sources-screen__toolbar">
        <Button variant="primary" icon="catalog" onClick={() => setAdding(true)}>
          {t.sourcesScreen.addCatalog}
        </Button>
        <Button icon="folder" onClick={() => void addFolder()}>
          {t.sourcesScreen.addFolder}
        </Button>
      </div>
      {actionError && <ErrorBanner error={actionError} />}
      <p role="status" className="fl-visually-hidden">
        {announcement}
      </p>
      <ul ref={listRef} className="fl-source-rows" aria-label={t.sourcesScreen.listLabel}>
        {sources.map((source) => {
          if (!isEditable(source)) {
            return (
              <li key={source.id} className="fl-source-row">
                <Icon name="library" />
                <div className="fl-source-row__text">
                  <span className="fl-source-row__name">{source.name}</span>
                  <span className="fl-source-row__detail">
                    {t.sourcesScreen.libraryNote}
                  </span>
                </div>
                <Badge>{t.sources.kind.library}</Badge>
              </li>
            );
          }
          const index = registered.findIndex((s) => s.id === source.id);
          return (
            <li
              key={source.id}
              className={
                "fl-source-row fl-source-row--draggable" +
                (dragId === source.id ? " fl-source-row--dragging" : "")
              }
              draggable
              onDragStart={(event) => {
                if (event.dataTransfer) event.dataTransfer.effectAllowed = "move";
                setDragId(source.id);
              }}
              onDragOver={(event) => {
                if (dragId) event.preventDefault();
              }}
              onDrop={(event) => {
                event.preventDefault();
                dropOn(source.id);
              }}
              onDragEnd={() => setDragId(null)}
            >
              <Icon name={source.type === "catalog" ? "catalog" : "folder"} />
              <div className="fl-source-row__text">
                <span className="fl-source-row__name">{source.name}</span>
                <span className="fl-source-row__detail">
                  {source.type === "catalog" ? source.url : source.path}
                </span>
              </div>
              <Badge>{t.sources.kind[source.type]}</Badge>
              <div className="fl-source-row__actions">
                <IconButton
                  icon="arrowUp"
                  size="sm"
                  label={t.sourcesScreen.moveUp(source.name)}
                  data-source-id={source.id}
                  data-move="up"
                  disabled={index === 0}
                  onClick={() => moveBy(source.id, -1)}
                />
                <IconButton
                  icon="arrowDown"
                  size="sm"
                  label={t.sourcesScreen.moveDown(source.name)}
                  data-source-id={source.id}
                  data-move="down"
                  disabled={index === registered.length - 1}
                  onClick={() => moveBy(source.id, 1)}
                />
                <IconButton
                  icon="pencil"
                  size="sm"
                  label={t.sourcesScreen.rename(source.name)}
                  onClick={() => setRenaming(source)}
                />
                <IconButton
                  icon="trash"
                  size="sm"
                  label={t.sourcesScreen.remove(source.name)}
                  onClick={() => setDeleting(source)}
                />
              </div>
            </li>
          );
        })}
      </ul>

      {adding && <AddCatalogDialog onClose={() => setAdding(false)} onAdded={onAdded} />}
      {renaming && (
        <RenameSourceDialog
          source={renaming}
          onClose={() => setRenaming(null)}
          onRenamed={(updated) => {
            setRenaming(null);
            setSources(
              (list) => list?.map((s) => (s.id === updated.id ? updated : s)) ?? null,
            );
            toast.show({
              message: t.sourcesScreen.renamed(updated.name),
              tone: "success",
            });
          }}
        />
      )}
      <Dialog
        open={deleting !== null}
        onClose={() => setDeleting(null)}
        role="alertdialog"
        title={t.sourcesScreen.deleteDialog.title}
        description={
          deleting ? t.sourcesScreen.deleteDialog.description(deleting.name) : ""
        }
        initialFocus={cancelDeleteRef}
        footer={
          <>
            <Button
              ref={cancelDeleteRef}
              variant="secondary"
              onClick={() => setDeleting(null)}
            >
              {t.common.cancel}
            </Button>
            <Button variant="danger" onClick={() => void confirmDelete()}>
              {t.sourcesScreen.deleteDialog.confirm}
            </Button>
          </>
        }
      />
    </div>
  );
}
