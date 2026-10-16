import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { getCatalogEntries } from "../../api/catalog";
import { toAppError, type AppError } from "../../api/error";
import { listSources, type Source } from "../../api/sources";
import { useRoute } from "../../router";
import { LibraryContext, type LibraryApi } from "./LibraryContext";
import { summarize, type SourceSummary } from "./sourceData";

/**
 * Tient la liste des sources (rechargée à chaque changement d'écran, pour suivre les
 * ajouts et suppressions faits dans « Sources ») et leur état pour la barre latérale.
 * Le nombre d'histoires d'un catalogue est lu dans son cache, sans réseau.
 */
export function LibraryProvider({ children }: { children: ReactNode }) {
  const route = useRoute();
  const [sources, setSources] = useState<Source[] | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [summaries, setSummaries] = useState<Record<string, SourceSummary>>({});
  const [reloads, setReloads] = useState(0);

  useEffect(() => {
    let active = true;
    listSources()
      .then((view) => {
        if (!active) return;
        setSources(view.sources);
        setError(null);
        for (const source of view.sources) {
          if (source.type !== "catalog") continue;
          getCatalogEntries(source.id)
            .then((catalog) => {
              if (!active || catalog.status.updated_at === null) return;
              const summary = summarize({
                entries: catalog.entries,
                warnings: catalog.warnings,
                offline: catalog.status.offline,
                updatedAt: catalog.status.updated_at,
                problem: catalog.status.error,
              });
              setSummaries((all) => ({ ...all, [source.id]: summary }));
            })
            .catch(() => undefined);
        }
      })
      .catch((caught) => {
        if (active) setError(toAppError(caught));
      });
    return () => {
      active = false;
    };
  }, [route, reloads]);

  const report = useCallback((sourceId: string, summary: SourceSummary) => {
    setSummaries((all) => ({ ...all, [sourceId]: summary }));
  }, []);
  const reload = useCallback(() => setReloads((n) => n + 1), []);

  const api = useMemo<LibraryApi>(
    () => ({ sources, error, summaries, report, reload }),
    [sources, error, summaries, report, reload],
  );
  return <LibraryContext.Provider value={api}>{children}</LibraryContext.Provider>;
}
