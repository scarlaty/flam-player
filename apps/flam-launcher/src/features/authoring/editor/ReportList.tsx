import type { FolderProblem } from "../../../api/authoring";
import type { EditorView } from "../../../api/editor";
import { Icon } from "../../../components";
import { t } from "../../../i18n";
import { problemText } from "../problemText";
import { nodeForPath } from "./model";

export interface ReportListProps {
  view: EditorView;
  /** Mène à l'élément concerné (sélection dans l'arbre). */
  onGoTo: (id: string) => void;
}

/** Rapport en bas de la fenêtre (§ 4.1) : chaque ligne mène à l'élément concerné. */
export function ReportList({ view, onGoTo }: ReportListProps) {
  const text = t.authoringEditor.report;
  const problems = view.report.problems;
  const blocking = problems.filter((p) => p.severity === "blocking");
  const warnings = problems.filter((p) => p.severity === "warning");

  const line = (problem: FolderProblem, index: number) => {
    const node = nodeForPath(view.report.root, problem.path);
    const label = problemText(problem);
    return (
      <li key={index}>
        <button
          type="button"
          className="fl-editor-report__line"
          aria-label={text.goTo(label)}
          onClick={() => onGoTo(node.id)}
        >
          <Icon name="alert" size={16} />
          <span>{label}</span>
        </button>
      </li>
    );
  };

  return (
    <section className="fl-editor-report" aria-labelledby="fl-editor-report-title">
      <h2 id="fl-editor-report-title" className="fl-editor-report__title">
        {text.heading}
      </h2>
      {problems.length === 0 && <p className="fl-editor-report__none">{text.none}</p>}
      {blocking.length > 0 && (
        <>
          <h3 className="fl-editor-report__group fl-editor-report__group--blocking">
            {text.blocking}
          </h3>
          <ul className="fl-editor-report__list">{blocking.map(line)}</ul>
        </>
      )}
      {warnings.length > 0 && (
        <>
          <h3 className="fl-editor-report__group">{text.warnings}</h3>
          <ul className="fl-editor-report__list">{warnings.map(line)}</ul>
        </>
      )}
    </section>
  );
}
