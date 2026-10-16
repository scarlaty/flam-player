import { useEffect } from "react";
import { Button, EmptyState } from "../../components";
import { t } from "../../i18n";
import { libraryHash, useSourceId } from "../../router";
import { ErrorBanner } from "../ErrorBanner";
import { useLibrary } from "./LibraryContext";
import { SourceView } from "./SourceView";
import type { StoryHooks } from "./storyState";

/**
 * Écran `#/library/<id>` : contenu de la source choisie (la première par défaut).
 * Les points d'accroche (`StoryHooks`) permettent aux fonctionnalités d'installation, de
 * lancement et de compatibilité de décorer les cartes sans modifier cet écran.
 */
export function LibraryScreen(hooks: StoryHooks) {
  const { sources, error, reload } = useLibrary();
  const requested = useSourceId();
  const { reopenEntry, onReopened } = hooks;
  // Source supprimée entre-temps : il n'y a plus de fiche à rouvrir.
  const reopenLost =
    reopenEntry !== undefined &&
    sources !== null &&
    !sources.some((candidate) => candidate.id === reopenEntry.sourceId);
  useEffect(() => {
    if (reopenLost) onReopened?.();
  }, [reopenLost, onReopened]);

  if (error && !sources) {
    return (
      <ErrorBanner
        error={error}
        action={<Button onClick={reload}>{t.errors.retry}</Button>}
      />
    );
  }
  if (!sources) return <p>{t.common.loading}</p>;

  const source =
    requested === null ? sources[0] : sources.find((s) => s.id === requested);
  if (!source) {
    return (
      <EmptyState
        icon="alert"
        title={t.library.sourceNotFound.title}
        description={t.library.sourceNotFound.description}
        action={
          <a className="fl-library__link" href={libraryHash()}>
            {t.library.sourceNotFound.action}
          </a>
        }
      />
    );
  }
  return <SourceView key={source.id} source={source} {...hooks} />;
}
