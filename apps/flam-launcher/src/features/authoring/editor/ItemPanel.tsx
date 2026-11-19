import { useId, useState, type FormEvent, type Ref } from "react";
import {
  AUDIO_EXTENSIONS,
  IMAGE_EXTENSIONS,
  deleteItem,
  keepComplement,
  moveItem,
  pickOneFile,
  renameItem,
  setFolderSettings,
  setImage,
  setStorySettings,
  setTitleAudio,
  type BookStart,
  type EditorMenu,
  type EditorNode,
  type EditorUpdate,
  type EditorView,
  type FolderFin,
  type Target,
} from "../../../api/editor";
import type { FolderMode } from "../../../api/authoring";
import { previewSpeech, stopSpeechPreview } from "../../../api/speech";
import {
  Badge,
  Button,
  CheckboxField,
  RadioGroup,
  SelectField,
  TextField,
} from "../../../components";
import { t } from "../../../i18n";
import { problemText } from "../problemText";
import {
  MAX_LABEL_CHARS,
  formatSize,
  imageOf,
  isWithin,
  menusOf,
  parentOf,
  problemsOf,
  shiftedOrder,
  targetOf,
} from "./model";
import { Thumbnail } from "./Thumbnail";

/** Lance une opération de l'éditeur (occupé, résultat annoncé, erreurs dites). */
export type RunOp = (action: () => Promise<EditorUpdate>) => Promise<boolean>;

export interface ItemPanelProps {
  view: EditorView;
  node: EditorNode;
  busy: boolean;
  run: RunOp;
  titleRef: Ref<HTMLInputElement>;
  onShift: (id: string, delta: -1 | 1) => void;
  onAskDelete: (id: string) => void;
  onAddFiles: (menu: string) => void;
  onNewMenu: (parent: string) => void;
}

/** Panneau de l'élément choisi (§ 4.1) : titre, image, titre lu, rangement, réglages. */
export function ItemPanel(props: ItemPanelProps) {
  const { view, node } = props;
  const text = t.authoringEditor.panel;
  const headingId = useId();
  const root = view.report.root;
  const isRoot = node.id === "";
  const mode =
    node.kind === "menu"
      ? node.mode
      : (parentOf(root, node.id)?.mode ?? view.report.mode);
  const chapters = mode === "chapitres";
  const heading = isRoot
    ? text.rootHeading
    : node.kind === "menu"
      ? chapters
        ? text.partHeading
        : text.menuHeading
      : chapters
        ? text.chapterHeading
        : text.storyHeading;
  const problems = problemsOf(view, node.id);
  // Formulaires remis à zéro quand l'élément ou son état changent.
  const formKey = `${node.id}|${node.title}|${JSON.stringify(view.story)}|${JSON.stringify(view.folders[node.id] ?? null)}|${node.kind === "menu" ? `${node.mode}|${node.annoncer}` : ""}`;

  return (
    <section className="fl-editor-panel" aria-labelledby={headingId}>
      <p className="fl-editor-panel__kind">{heading}</p>
      <h2 id={headingId} className="fl-editor-panel__title">
        {node.title}
      </h2>

      {problems.length > 0 && (
        <div className="fl-editor-panel__section">
          <h3>{text.problems}</h3>
          <ul className="fl-editor-panel__problems">
            {problems.map((problem, index) => (
              <li key={index}>{problemText(problem)}</li>
            ))}
          </ul>
        </div>
      )}

      {isRoot ? (
        <StoryDetails key={formKey} {...props} />
      ) : (
        <TitleForm key={formKey} {...props} />
      )}

      <ImageSection {...props} />
      <TitleAudioSection {...props} />

      {node.kind === "story" && (
        <div className="fl-editor-panel__section">
          <h3>{text.audio}</h3>
          <p>{text.audioFile(node.file, formatSize(node.bytes))}</p>
        </div>
      )}

      {!isRoot && <Organize {...props} />}

      {node.kind === "menu" && (
        <div className="fl-editor-panel__section">
          <div className="fl-editor-panel__actions">
            <Button
              icon="plus"
              size="sm"
              disabled={props.busy}
              onClick={() => props.onAddFiles(node.id)}
            >
              {text.addHere}
            </Button>
            <Button
              icon="folder"
              size="sm"
              variant="secondary"
              disabled={props.busy}
              onClick={() => props.onNewMenu(node.id)}
            >
              {text.newMenuHere}
            </Button>
          </div>
          <FolderSettings key={formKey} {...props} node={node} />
        </div>
      )}
    </section>
  );
}

function target(view: EditorView, id: string): Target | null {
  return targetOf(view, id);
}

/** Titre affiché d'un élément (et, en option, nom du fichier). */
function TitleForm({ view, node, busy, run, titleRef }: ItemPanelProps) {
  const text = t.authoringEditor.panel;
  const [title, setTitle] = useState(node.title);
  const [renameFile, setRenameFile] = useState(false);
  const tooLong = [...title.trim()].length > MAX_LABEL_CHARS;

  function submit(event: FormEvent) {
    event.preventDefault();
    const at = target(view, node.id);
    if (!at) return;
    void run(() => renameItem(at, title, renameFile));
  }

  return (
    <form className="fl-editor-panel__section" onSubmit={submit}>
      <TextField
        ref={titleRef}
        label={text.title}
        value={title}
        maxLength={200}
        onChange={(event) => setTitle(event.target.value)}
        hint={
          tooLong ? text.titleTooLong(MAX_LABEL_CHARS) : text.titleHint(MAX_LABEL_CHARS)
        }
      />
      <CheckboxField
        label={text.renameFile}
        hint={text.renameFileHint}
        checked={renameFile}
        onChange={(event) => setRenameFile(event.target.checked)}
      />
      <div className="fl-editor-panel__actions">
        <Button
          type="submit"
          size="sm"
          variant="primary"
          disabled={busy || (title === node.title && !renameFile)}
        >
          {text.saveTitle}
        </Button>
      </div>
    </form>
  );
}

/** Titre, sous-titre, description et âge de l'histoire (`_histoire.json`). */
function StoryDetails({ view, busy, run, titleRef }: ItemPanelProps) {
  const text = t.authoringEditor.panel;
  const story = view.story;
  const [titre, setTitre] = useState(story.titre ?? view.report.title);
  const [sousTitre, setSousTitre] = useState(story.sous_titre ?? "");
  const [description, setDescription] = useState(story.description ?? "");
  const [age, setAge] = useState(story.age === null ? "" : String(story.age));
  const descriptionId = useId();
  const ageValue = age.trim() === "" ? null : Number(age);
  const ageInvalid =
    ageValue !== null && (!Number.isInteger(ageValue) || ageValue < 0 || ageValue > 99);

  function submit(event: FormEvent) {
    event.preventDefault();
    if (ageInvalid) return;
    const clean = (value: string) => (value.trim() === "" ? null : value);
    void run(() =>
      setStorySettings({
        titre: clean(titre),
        sousTitre: clean(sousTitre),
        description: clean(description),
        age: ageValue,
      }),
    );
  }

  return (
    <form className="fl-editor-panel__section" onSubmit={submit}>
      <TextField
        ref={titleRef}
        label={text.storyTitle}
        value={titre}
        maxLength={200}
        onChange={(event) => setTitre(event.target.value)}
        hint={
          [...titre.trim()].length > MAX_LABEL_CHARS
            ? text.titleTooLong(MAX_LABEL_CHARS)
            : undefined
        }
      />
      <TextField
        label={text.subtitle}
        value={sousTitre}
        maxLength={200}
        onChange={(event) => setSousTitre(event.target.value)}
      />
      <div className="fl-editor-panel__field">
        <label htmlFor={descriptionId}>{text.description}</label>
        <textarea
          id={descriptionId}
          className="fl-editor-panel__textarea"
          rows={3}
          maxLength={2000}
          value={description}
          onChange={(event) => setDescription(event.target.value)}
        />
      </div>
      <TextField
        label={text.age}
        inputMode="numeric"
        value={age}
        maxLength={2}
        onChange={(event) => setAge(event.target.value)}
        hint={text.ageHint}
        error={ageInvalid ? text.ageInvalid : null}
      />
      <div className="fl-editor-panel__actions">
        <Button type="submit" size="sm" variant="primary" disabled={busy || ageInvalid}>
          {text.saveDetails}
        </Button>
      </div>
    </form>
  );
}

function ImageSection({ view, node, busy, run }: ItemPanelProps) {
  const text = t.authoringEditor.panel;
  const shown = imageOf(node);
  const isRoot = node.id === "";
  const auto = isRoot
    ? shown.id
      ? text.autoImageCover
      : text.autoImageNone
    : node.kind === "menu"
      ? shown.id
        ? text.autoImageMenu
        : text.autoImageNone
      : text.autoImageStory;

  async function choose() {
    const at = target(view, node.id);
    if (!at) return;
    const source = await pickOneFile(
      t.authoringEditor.pick.image,
      t.authoringEditor.pick.imageFilter,
      IMAGE_EXTENSIONS,
    );
    if (source !== null) await run(() => setImage(at, source));
  }

  return (
    <div className="fl-editor-panel__section">
      <h3>{isRoot ? text.coverImage : text.image}</h3>
      <div className="fl-editor-panel__media">
        <Thumbnail
          id={shown.id}
          stamp={shown.id ? view.stamps[shown.id] : undefined}
          size={200}
          alt={text.imageAlt(node.title)}
          fallback={node.kind === "menu" ? "folder" : "play"}
          className="fl-editor-panel__image"
        />
        <div className="fl-editor-panel__media-text">
          {shown.automatic && (
            <p>
              <Badge tone="neutral">{text.automatic}</Badge> {auto}
            </p>
          )}
          <div className="fl-editor-panel__actions">
            <Button size="sm" disabled={busy} onClick={() => void choose()}>
              {text.chooseImage}
            </Button>
            {shown.keepable && (
              <Button
                size="sm"
                variant="secondary"
                title={text.keepHelp}
                disabled={busy}
                onClick={() => {
                  const at = target(view, node.id);
                  if (at) void run(() => keepComplement(at));
                }}
              >
                {text.keep}
              </Button>
            )}
            {node.image && (
              <Button
                size="sm"
                variant="danger-quiet"
                disabled={busy}
                onClick={() => {
                  const at = node.image ? target(view, node.image) : null;
                  if (at) void run(() => deleteItem(at));
                }}
              >
                {text.removeImage}
              </Button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function TitleAudioSection({ view, node, busy, run }: ItemPanelProps) {
  const text = t.authoringEditor.panel;
  const [speaking, setSpeaking] = useState(false);
  const [voiceError, setVoiceError] = useState(false);
  const file = node.title_audio;

  async function choose() {
    const at = target(view, node.id);
    if (!at) return;
    const source = await pickOneFile(
      t.authoringEditor.pick.audio,
      t.authoringEditor.pick.audioFilter,
      AUDIO_EXTENSIONS,
    );
    if (source !== null) await run(() => setTitleAudio(at, source));
  }

  async function listen() {
    setVoiceError(false);
    if (speaking) {
      setSpeaking(false);
      await stopSpeechPreview().catch(() => undefined);
      return;
    }
    try {
      setSpeaking(true);
      const info = await previewSpeech(node.title);
      if (info.played) {
        window.setTimeout(() => setSpeaking(false), Math.max(500, info.duration_ms));
      } else setSpeaking(false);
    } catch {
      setSpeaking(false);
      setVoiceError(true);
    }
  }

  return (
    <div className="fl-editor-panel__section">
      <h3>{text.titleAudio}</h3>
      {file ? (
        <p>{text.titleAudioFile(file.split("/").pop() ?? file)}</p>
      ) : (
        <p>
          <Badge tone="neutral">{text.automatic}</Badge> {text.titleAudioVoice}
        </p>
      )}
      {voiceError && (
        <p className="fl-editor-panel__note" role="status">
          {text.voiceUnavailable}
        </p>
      )}
      <div className="fl-editor-panel__actions">
        {!file && (
          <Button
            size="sm"
            icon="play"
            aria-pressed={speaking}
            onClick={() => void listen()}
          >
            {speaking ? text.stopVoice : text.listenVoice}
          </Button>
        )}
        <Button
          size="sm"
          variant={file ? "secondary" : "ghost"}
          disabled={busy}
          onClick={() => void choose()}
        >
          {file ? text.replaceAudio : text.chooseAudio}
        </Button>
        {file && (
          <Button
            size="sm"
            variant="danger-quiet"
            disabled={busy}
            onClick={() => {
              const at = target(view, file);
              if (at) void run(() => deleteItem(at));
            }}
          >
            {text.removeAudio}
          </Button>
        )}
      </div>
    </div>
  );
}

/** Monter, descendre, déplacer vers un autre menu, mettre à la corbeille. */
function Organize({ view, node, busy, run, onShift, onAskDelete }: ItemPanelProps) {
  const text = t.authoringEditor.panel;
  const root = view.report.root;
  const parent = parentOf(root, node.id);
  const destinations = menusOf(root).filter(
    (menu) =>
      menu.id !== parent?.id &&
      !(node.kind === "menu" && isWithin(menu.id, node.id) && menu.id !== ""),
  );
  const [dest, setDest] = useState(destinations[0]?.id ?? "");
  const canUp = parent !== null && shiftedOrder(parent, node.id, -1) !== null;
  const canDown = parent !== null && shiftedOrder(parent, node.id, 1) !== null;
  const chosen = destinations.some((d) => d.id === dest)
    ? dest
    : (destinations[0]?.id ?? "");

  return (
    <div className="fl-editor-panel__section">
      <h3>{text.organize}</h3>
      <div className="fl-editor-panel__actions">
        <Button
          size="sm"
          icon="arrowUp"
          disabled={busy || !canUp}
          onClick={() => onShift(node.id, -1)}
        >
          {text.moveUp}
        </Button>
        <Button
          size="sm"
          icon="arrowDown"
          disabled={busy || !canDown}
          onClick={() => onShift(node.id, 1)}
        >
          {text.moveDown}
        </Button>
      </div>
      {destinations.length > 0 && (
        <div className="fl-editor-panel__inline">
          <SelectField
            label={text.moveTo}
            value={chosen}
            options={destinations.map((d) => ({ value: d.id, label: d.label }))}
            onChange={(event) => setDest(event.target.value)}
          />
          <Button
            size="sm"
            disabled={busy || chosen === ""}
            onClick={() => {
              const at = target(view, node.id);
              if (at) void run(() => moveItem(at, chosen));
            }}
          >
            {text.move}
          </Button>
        </div>
      )}
      <div className="fl-editor-panel__actions">
        <Button
          size="sm"
          icon="trash"
          variant="danger-quiet"
          disabled={busy}
          onClick={() => onAskDelete(node.id)}
        >
          {text.delete}
        </Button>
      </div>
    </div>
  );
}

type ModeChoice = FolderMode | "inherit";
type FinChoice = FolderFin | "inherit";

/**
 * Mode, fin, début et annonce d'un dossier (racine : `_histoire.json`). Les choix restent
 * locaux jusqu'à « Appliquer » : parcourir les options au clavier n'écrit rien (une seule
 * écriture, une seule entrée du journal) et les contrôles ne sont jamais désactivés sous le
 * focus.
 */
function FolderSettings({
  view,
  node,
  busy,
  run,
}: ItemPanelProps & { node: EditorMenu }) {
  const text = t.authoringEditor.panel;
  const isRoot = node.id === "";
  const fields = isRoot
    ? null
    : (view.folders[node.id] ?? { mode: null, fin: null, annoncer: null });
  const initialMode: ModeChoice = isRoot ? node.mode : (fields?.mode ?? "inherit");
  const initialFin: FinChoice = isRoot
    ? (view.story.fin ?? "menu")
    : (fields?.fin ?? "inherit");
  const initialDebut: BookStart = view.story.debut ?? "chapitre";
  const [mode, setMode] = useState<ModeChoice>(initialMode);
  const [fin, setFin] = useState<FinChoice>(initialFin);
  const [debut, setDebut] = useState<BookStart>(initialDebut);
  const [annoncer, setAnnoncer] = useState(node.annoncer);
  const inherited = parentOf(view.report.root, node.id)?.mode ?? view.report.mode;
  const chapters = (mode === "inherit" ? inherited : mode) === "chapitres";

  const modeOptions: { value: ModeChoice; label: string }[] = [
    ...(isRoot ? [] : [{ value: "inherit" as const, label: text.modeInherit }]),
    { value: "recueil", label: text.modeRecueil },
    { value: "chapitres", label: text.modeChapitres },
  ];
  const finOptions: { value: FinChoice; label: string }[] = [
    ...(isRoot ? [] : [{ value: "inherit" as const, label: text.finInherit }]),
    { value: "menu", label: chapters ? text.finMenuChapitres : text.finMenuRecueil },
    chapters
      ? { value: "fin", label: text.finFin }
      : { value: "suivante", label: text.finSuivante },
  ];
  // Fin qui n'a pas de sens dans le mode choisi : ramenée à « menu ».
  const finShown = finOptions.some((o) => o.value === fin) ? fin : "menu";

  const patch: {
    mode?: FolderMode | null;
    fin?: FolderFin | null;
    annoncer?: boolean | null;
    debut?: BookStart | null;
  } = {};
  if (mode !== initialMode) patch.mode = mode === "inherit" ? null : mode;
  if (finShown !== initialFin) patch.fin = finShown === "inherit" ? null : finShown;
  if (chapters && annoncer !== node.annoncer) patch.annoncer = annoncer;
  if (isRoot && chapters && debut !== initialDebut) patch.debut = debut;
  const changed = Object.keys(patch).length > 0;

  function submit(event: FormEvent) {
    event.preventDefault();
    if (!changed || busy) return;
    void run(() =>
      isRoot ? setStorySettings(patch) : setFolderSettings(node.id, patch),
    );
  }

  return (
    <form className="fl-editor-panel__settings" onSubmit={submit}>
      <h3>{text.settings}</h3>
      <RadioGroup<ModeChoice>
        legend={text.mode}
        value={mode}
        options={modeOptions}
        onChange={setMode}
      />
      {isRoot && chapters && (
        <RadioGroup<BookStart>
          legend={text.debut}
          value={debut}
          options={[
            { value: "chapitre", label: text.debutChapitre },
            { value: "menu", label: text.debutMenu },
          ]}
          onChange={setDebut}
        />
      )}
      <SelectField
        label={text.fin}
        value={finShown}
        options={finOptions}
        onChange={(event) => setFin(event.target.value as FinChoice)}
      />
      {chapters && (
        <CheckboxField
          label={text.annoncer}
          hint={text.annoncerHint}
          checked={annoncer}
          onChange={(event) => setAnnoncer(event.target.checked)}
        />
      )}
      <div className="fl-editor-panel__actions">
        <Button
          type="submit"
          size="sm"
          variant="primary"
          aria-disabled={!changed || busy || undefined}
        >
          {text.applySettings}
        </Button>
      </div>
    </form>
  );
}
