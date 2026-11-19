import type { EditorView } from "../../../api/editor";
import { Badge, Button } from "../../../components";
import { t } from "../../../i18n";
import { imageOf } from "./model";
import { Thumbnail } from "./Thumbnail";

export interface EditorHeaderProps {
  view: EditorView;
  busy: boolean;
  creating: boolean;
  onSelectRoot: () => void;
  onReveal: () => void;
  onUndo: () => void;
  onRedo: () => void;
  onCreate: () => void;
}

/**
 * En-tête (§ 4.1) : couverture, titre, mode, âge et description de l'histoire (modifiables en
 * choisissant « L'histoire » dans l'arbre), puis les actions générales.
 */
export function EditorHeader(props: EditorHeaderProps) {
  const { view, busy } = props;
  const text = t.authoringEditor.header;
  const report = view.report;
  const cover = imageOf(report.root);
  const updating = report.uuid !== null && report.installed_version !== null;
  const chapters = report.mode === "chapitres";
  const journal = view.journal;
  const blockedId = "fl-editor-blocked-help";

  return (
    <header className="fl-editor__header">
      <button
        type="button"
        className="fl-editor__cover"
        onClick={props.onSelectRoot}
        aria-label={text.cover}
      >
        <Thumbnail
          id={cover.id}
          stamp={cover.id ? view.stamps[cover.id] : undefined}
          size={72}
          alt=""
          fallback="book"
        />
      </button>
      <div className="fl-editor__identity">
        <h1 className="fl-editor__title">{report.title}</h1>
        <p className="fl-editor__facts">
          <Badge tone="info">{chapters ? text.modeChapitres : text.modeRecueil}</Badge>
          <span>{text.count(report.story_count, chapters)}</span>
          {view.story.age !== null && <span>{text.age(view.story.age)}</span>}
        </p>
        {view.story.description && (
          <p className="fl-editor__description">{view.story.description}</p>
        )}
      </div>
      <div className="fl-editor__actions">
        <Button size="sm" variant="ghost" icon="folder" onClick={props.onReveal}>
          {text.reveal}
        </Button>
        <Button
          size="sm"
          variant="secondary"
          title={text.undoHelp(journal.undo_label)}
          aria-keyshortcuts="Control+Z"
          disabled={busy || !journal.can_undo}
          onClick={props.onUndo}
        >
          {text.undo}
        </Button>
        <Button
          size="sm"
          variant="secondary"
          title={text.redoHelp(journal.redo_label)}
          aria-keyshortcuts="Control+Y"
          disabled={busy || !journal.can_redo}
          onClick={props.onRedo}
        >
          {text.redo}
        </Button>
        <Button
          variant="accent"
          icon="sparkle"
          aria-describedby={report.blocked ? blockedId : undefined}
          aria-busy={props.creating || undefined}
          disabled={busy || report.blocked}
          onClick={props.onCreate}
        >
          {props.creating ? text.creating : updating ? text.update : text.create}
        </Button>
        {report.blocked && (
          <span id={blockedId} className="fl-editor__blocked">
            {text.blockedHelp}
          </span>
        )}
      </div>
    </header>
  );
}
