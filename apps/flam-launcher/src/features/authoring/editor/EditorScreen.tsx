import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import type { ConflictChoice } from "../../../api/authoring";
import {
  AUDIO_EXTENSIONS,
  IMAGE_EXTENSIONS,
  createFromEditor,
  deleteItem,
  editorView,
  emptyTrash,
  importFiles,
  newMenu,
  pickFiles,
  redo,
  reorder,
  revealFolder,
  undo,
  type EditorUpdate,
  type EditorView,
} from "../../../api/editor";
import { toAppError } from "../../../api/error";
import { Banner, Button, Dialog, RadioGroup, TextField } from "../../../components";
import { t } from "../../../i18n";
import { EditorHeader } from "./EditorHeader";
import { useEditorDnd } from "./dnd/useEditorDnd";
import { EditorTree } from "./EditorTree";
import { editorErrorText } from "./errors";
import { FolderMissingBanner } from "./FolderMissingBanner";
import { ItemPanel } from "./ItemPanel";
import {
  findNode,
  isWithin,
  nodeForPath,
  parentOf,
  shiftedOrder,
  targetOf,
} from "./model";
import { ReportList } from "./ReportList";
import { useFolderWatch } from "./useFolderWatch";
import "./Editor.css";

type Ask =
  | { kind: "delete"; id: string; title: string }
  | { kind: "newMenu"; parent: string }
  | { kind: "emptyTrash" }
  | { kind: "conflict" };

/** Vrai si le focus est dans un champ de saisie (Ctrl+Z y reste celui du champ). */
function inTextField(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.isContentEditable ||
    target.tagName === "TEXTAREA" ||
    (target.tagName === "INPUT" &&
      !["checkbox", "radio", "button", "submit"].includes(
        (target as HTMLInputElement).type,
      ))
  );
}

/**
 * Fenêtre de l'éditeur d'histoires (§ 4.1) : en-tête, arbre à gauche, panneau de l'élément à
 * droite, rapport en bas. Chaque opération est faite par le backend sur le dossier (ids
 * seulement) et rend la vue relue ; Annuler et Rétablir parcourent le journal de la session.
 */
export function EditorScreen() {
  const text = t.authoringEditor;
  const [view, setView] = useState<EditorView | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState("");
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [creating, setCreating] = useState(false);
  const [status, setStatus] = useState("");
  const [refocus, setRefocus] = useState(0);
  const statusTimer = useRef<number | undefined>(undefined);

  /** Annonce `message`, même identique au précédent (la région est vidée puis réécrite). */
  const announce = useCallback((message: string) => {
    setStatus("");
    window.clearTimeout(statusTimer.current);
    statusTimer.current = window.setTimeout(() => setStatus(message), 60);
  }, []);
  useEffect(() => () => window.clearTimeout(statusTimer.current), []);
  const [error, setError] = useState<string | null>(null);
  const [ask, setAsk] = useState<Ask | null>(null);
  const [menuName, setMenuName] = useState("");
  const [choice, setChoice] = useState<ConflictChoice>("new_story");
  const titleRef = useRef<HTMLInputElement>(null);
  const busyRef = useRef(false);

  const load = useCallback(async () => {
    try {
      setLoadError(null);
      setView(await editorView());
    } catch (caught) {
      setLoadError(editorErrorText(toAppError(caught)));
    }
  }, []);

  useEffect(() => {
    let alive = true;
    editorView().then(
      (loaded) => {
        if (alive) setView(loaded);
      },
      (caught: unknown) => {
        if (alive) setLoadError(editorErrorText(toAppError(caught)));
      },
    );
    return () => {
      alive = false;
    };
  }, []);

  // Changement fait ailleurs : vue relue, sélection gardée si l'élément existe encore, sinon son
  // menu le plus proche (lot 9). Dossier disparu : lecture seule.
  const onWatchedView = useCallback((next: EditorView) => {
    setView(next);
    setSelectedId((id) => nodeForPath(next.report.root, id).id);
  }, []);
  const { missing } = useFolderWatch({ busy, onView: onWatchedView, onNotice: announce });
  const missingRef = useRef(false);
  useEffect(() => {
    missingRef.current = missing;
  }, [missing]);
  const locked = busy || missing;

  // Sélection gardée si l'élément existe encore, sinon la racine.
  const node = view ? (findNode(view.report.root, selectedId) ?? view.report.root) : null;

  /** Lance une opération : une à la fois, résultat annoncé, erreur dite puis vue relue. */
  const run = useCallback(
    async (action: () => Promise<EditorUpdate>, say = text.done): Promise<boolean> => {
      if (busyRef.current || missingRef.current) return false;
      busyRef.current = true;
      setBusy(true);
      setError(null);
      try {
        const update = await action();
        setView(update.view);
        // Une image ou un titre lu rend son propre id : la sélection va à son élément.
        const first = update.ids[0];
        if (first !== undefined) {
          const chosen = nodeForPath(update.view.report.root, first).id;
          setSelectedId(chosen);
          // Ses menus sont dépliés : la ligne choisie reste visible et atteignable au clavier
          // (déplacement ou copie dans un menu replié).
          setCollapsed((current) => {
            const next = new Set(
              [...current].filter((menu) => !isWithin(chosen, menu) || menu === chosen),
            );
            return next.size === current.size ? current : next;
          });
        }
        const skipped =
          update.skipped.length > 0 ? ` ${text.skipped(update.skipped.length)}` : "";
        const said =
          update.trashed !== null ? text.trash.done(update.trashed) : say(update.label);
        announce(`${said}${skipped}`);
        return true;
      } catch (caught) {
        setError(editorErrorText(toAppError(caught)));
        await load();
        return false;
      } finally {
        busyRef.current = false;
        setBusy(false);
      }
    },
    [load, text, announce],
  );

  const doUndo = useCallback(() => void run(undo, text.undone), [run, text]);
  const doRedo = useCallback(() => void run(redo, text.redone), [run, text]);

  // Ctrl+Z, Ctrl+Y (et Ctrl+Maj+Z) hors des champs de saisie.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.altKey) return;
      if (inTextField(event.target) || ask !== null) return;
      const key = event.key.toLowerCase();
      if (key === "z" && !event.shiftKey) {
        event.preventDefault();
        if (view?.journal.can_undo) doUndo();
      } else if (key === "y" || (key === "z" && event.shiftKey)) {
        event.preventDefault();
        if (view?.journal.can_redo) doRedo();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [view, ask, doUndo, doRedo]);

  // Glisser-déposer (lot 8) : fichiers de l'Explorateur et déplacement au pointeur.
  const refocusTree = useCallback(() => setRefocus((n) => n + 1), []);
  const { attachRoot, ...dnd } = useEditorDnd({
    view,
    selectedId: node?.id ?? "",
    collapsed,
    busy,
    blocked: ask !== null,
    run,
    select: setSelectedId,
    refocusTree,
    announce,
    setError,
  });

  if (view === null || node === null) {
    return (
      <main className="fl-editor fl-editor--loading">
        {loadError ? (
          <Banner
            tone="danger"
            action={<Button onClick={() => void load()}>{text.retry}</Button>}
          >
            <p>{loadError}</p>
          </Banner>
        ) : (
          <p role="status">{text.loading}</p>
        )}
      </main>
    );
  }

  const root = view.report.root;

  /** Monte ou descend `id` ; depuis l'arbre (Alt+Flèche), le focus revient à sa ligne. */
  function shift(id: string, delta: -1 | 1, fromTree = false) {
    if (!view) return;
    const parent = parentOf(root, id);
    const order = parent ? shiftedOrder(parent, id, delta) : null;
    if (!parent || !order) return;
    setSelectedId(id);
    void run(() => reorder(parent.id, order)).then(() => {
      if (fromTree) setRefocus((n) => n + 1);
    });
  }

  /** Choisit `id` en dépliant ses menus : la ligne reste visible et atteignable au Tab. */
  function select(id: string) {
    setSelectedId(id);
    setCollapsed((current) => {
      const next = new Set(
        [...current].filter((menu) => !isWithin(id, menu) || menu === id),
      );
      return next.size === current.size ? current : next;
    });
  }

  async function addFiles(menu: string) {
    try {
      const sources = await pickFiles(text.pick.files, text.pick.mediaFilter, [
        ...AUDIO_EXTENSIONS,
        ...IMAGE_EXTENSIONS,
      ]);
      if (sources.length > 0) await run(() => importFiles(menu, sources));
    } catch (caught) {
      setError(editorErrorText(toAppError(caught)));
    }
  }

  /** Menu où ajouter : l'élément choisi s'il est un menu, sinon son menu. */
  const currentMenu =
    node.kind === "menu" ? node.id : (parentOf(root, node.id)?.id ?? "");

  function askDelete(id: string) {
    const found = findNode(root, id);
    if (found && id !== "") setAsk({ kind: "delete", id, title: found.title });
  }

  async function confirmDelete(id: string) {
    setAsk(null);
    const at = view ? targetOf(view, id) : null;
    if (!at) return;
    const parent = parentOf(root, id);
    if (await run(() => deleteItem(at))) {
      setSelectedId(parent?.id ?? "");
      setRefocus((n) => n + 1);
    }
  }

  function submitNewMenu(event: FormEvent, parent: string) {
    event.preventDefault();
    const name = menuName.trim() || text.panel.newMenuDefault;
    setAsk(null);
    void run(() => newMenu(parent, name));
  }

  async function create(answer: ConflictChoice | null) {
    setAsk(null);
    setCreating(true);
    const ok = await run(
      async () => {
        const result = await createFromEditor(answer);
        return { label: "", ids: [], skipped: [], trashed: null, view: result.view };
      },
      () => text.header.started,
    );
    setCreating(false);
    return ok;
  }

  function startCreate() {
    if (!view) return;
    const conflict = view.report.conflict;
    if (conflict?.kind === "copied") {
      setChoice("new_story");
      setAsk({ kind: "conflict" });
    } else {
      void create(conflict === null ? null : "new_story");
    }
  }

  const askTitle =
    ask?.kind === "delete"
      ? text.panel.deleteConfirmTitle(ask.title)
      : ask?.kind === "newMenu"
        ? text.panel.newMenuName
        : ask?.kind === "emptyTrash"
          ? text.trash.confirmTitle
          : text.header.copiedTitle;
  const conflict = view.report.conflict;

  return (
    <main
      ref={attachRoot}
      className={`fl-editor${dnd.filesOver ? " fl-editor--files-over" : ""}${dnd.tree.draggingId !== null ? " fl-editor--dragging" : ""}`}
    >
      <EditorHeader
        view={view}
        busy={locked}
        creating={creating}
        onSelectRoot={() => setSelectedId("")}
        onReveal={() => {
          revealFolder().catch((caught: unknown) =>
            setError(editorErrorText(toAppError(caught))),
          );
        }}
        onUndo={doUndo}
        onRedo={doRedo}
        onCreate={startCreate}
      />
      <div className="fl-editor__status" role="status" aria-live="polite">
        {status}
      </div>
      {missing && (
        <FolderMissingBanner
          uuid={view.report.installed_version !== null ? view.report.uuid : null}
          onError={setError}
        />
      )}
      {error && (
        <Banner tone="danger" role="alert" className="fl-editor__error">
          <p>{error}</p>
        </Banner>
      )}
      <div className="fl-editor__body">
        <nav className="fl-editor__side" aria-label={text.tree.label}>
          <div className="fl-editor__toolbar">
            <Button
              size="sm"
              icon="plus"
              disabled={locked}
              onClick={() => void addFiles(currentMenu)}
            >
              {text.tree.addFiles}
            </Button>
            <Button
              size="sm"
              variant="secondary"
              icon="folder"
              disabled={locked}
              onClick={() => {
                setMenuName("");
                setAsk({ kind: "newMenu", parent: currentMenu });
              }}
            >
              {text.tree.newMenu}
            </Button>
          </div>
          <div className="fl-editor__tree" data-dnd-zone="tree">
            <EditorTree
              dnd={dnd.tree}
              view={view}
              selectedId={node.id}
              onSelect={setSelectedId}
              collapsed={collapsed}
              refocus={refocus}
              onToggle={(id) => {
                // Replier le menu de la ligne choisie : la sélection remonte sur ce menu.
                if (!collapsed.has(id) && id !== node.id && isWithin(node.id, id))
                  setSelectedId(id);
                setCollapsed((current) => {
                  const next = new Set(current);
                  if (!next.delete(id)) next.add(id);
                  return next;
                });
              }}
              onShift={(id, delta) => shift(id, delta, true)}
              onDelete={askDelete}
              onRename={(id) => {
                setSelectedId(id);
                window.setTimeout(() => titleRef.current?.focus(), 0);
              }}
            />
          </div>
          <Button
            size="sm"
            variant="ghost"
            icon="trash"
            disabled={locked}
            onClick={() => setAsk({ kind: "emptyTrash" })}
          >
            {text.tree.emptyTrash}
          </Button>
        </nav>
        <div className="fl-editor__panel">
          <ItemPanel
            key={node.id}
            dropZone={dnd.panelZone}
            view={view}
            node={node}
            busy={locked}
            run={run}
            titleRef={titleRef}
            onShift={shift}
            onAskDelete={askDelete}
            onAddFiles={(menu) => void addFiles(menu)}
            onNewMenu={(parent) => {
              setMenuName("");
              setAsk({ kind: "newMenu", parent });
            }}
          />
        </div>
      </div>
      <ReportList view={view} onGoTo={select} />
      {dnd.overlay}

      <Dialog
        open={ask !== null}
        onClose={() => setAsk(null)}
        title={askTitle}
        role={
          ask?.kind === "delete" || ask?.kind === "emptyTrash" ? "alertdialog" : "dialog"
        }
        description={
          ask?.kind === "delete"
            ? text.panel.deleteConfirm
            : ask?.kind === "emptyTrash"
              ? text.trash.confirm
              : undefined
        }
        footer={
          ask?.kind === "delete" ? (
            <>
              <Button variant="ghost" onClick={() => setAsk(null)}>
                {text.panel.cancel}
              </Button>
              <Button variant="danger" onClick={() => void confirmDelete(ask.id)}>
                {text.panel.delete}
              </Button>
            </>
          ) : ask?.kind === "emptyTrash" ? (
            <>
              <Button variant="ghost" onClick={() => setAsk(null)}>
                {text.trash.cancel}
              </Button>
              <Button
                variant="danger"
                onClick={() => {
                  setAsk(null);
                  void run(emptyTrash);
                }}
              >
                {text.trash.empty}
              </Button>
            </>
          ) : ask?.kind === "newMenu" ? (
            <>
              <Button variant="ghost" onClick={() => setAsk(null)}>
                {text.panel.cancel}
              </Button>
              <Button variant="primary" type="submit" form="fl-editor-new-menu">
                {text.panel.create}
              </Button>
            </>
          ) : ask?.kind === "conflict" ? (
            <>
              <Button variant="ghost" onClick={() => setAsk(null)}>
                {text.header.cancel}
              </Button>
              <Button variant="primary" onClick={() => void create(choice)}>
                {text.header.confirmCreate}
              </Button>
            </>
          ) : null
        }
      >
        {ask?.kind === "newMenu" && (
          <form
            id="fl-editor-new-menu"
            onSubmit={(event) => submitNewMenu(event, ask.parent)}
          >
            <TextField
              label={text.panel.newMenuName}
              value={menuName}
              placeholder={text.panel.newMenuDefault}
              maxLength={120}
              autoFocus
              onChange={(event) => setMenuName(event.target.value)}
            />
          </form>
        )}
        {ask?.kind === "conflict" && conflict?.kind === "copied" && (
          <RadioGroup
            legend={text.header.copiedLegend}
            value={choice}
            onChange={setChoice}
            options={[
              { value: "new_story", label: text.header.newStory },
              { value: "replace_other", label: text.header.replace(conflict.title) },
            ]}
          />
        )}
      </Dialog>
    </main>
  );
}
