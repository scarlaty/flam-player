import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { findCompat, getCompatReport, type CompatReport } from "../../api/compat";
import type { StoryEntry } from "../../api/story";
import { useRoute } from "../../router";
import { CompatContext, type CompatApi } from "./CompatContext";
import { describeCompat } from "./compatInfo";

/**
 * Charge le rapport de compatibilité (campagne + conversions locales) et le relit à chaque
 * changement d'écran. Un échec de lecture n'est pas bloquant : les histoires sont alors
 * présentées comme non testées.
 */
export function CompatProvider({ children }: { children: ReactNode }) {
  const route = useRoute();
  const [report, setReport] = useState<CompatReport | null>(null);
  const [failed, setFailed] = useState(false);
  const [reloads, setReloads] = useState(0);

  useEffect(() => {
    let active = true;
    getCompatReport()
      .then((next) => {
        if (!active) return;
        setReport(next);
        setFailed(false);
      })
      .catch(() => {
        if (active) setFailed(true);
      });
    return () => {
      active = false;
    };
  }, [route, reloads]);

  const reload = useCallback(() => setReloads((n) => n + 1), []);
  const compatFor = useCallback(
    (entry: StoryEntry) => {
      if (!report && !failed) return undefined;
      return describeCompat(report ? findCompat(report.entries, entry) : undefined);
    },
    [report, failed],
  );

  const api = useMemo<CompatApi>(
    () => ({ issue: report?.issue ?? null, compatFor, reload }),
    [report, compatFor, reload],
  );
  return <CompatContext.Provider value={api}>{children}</CompatContext.Provider>;
}
