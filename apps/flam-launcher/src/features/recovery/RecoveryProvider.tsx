import { useCallback, useMemo, useState, type ReactNode } from "react";
import { ackRecovery, type Recovery } from "../../api/recovery";
import { RecoveryContext, type RecoveryApi } from "./RecoveryContext";

/** Garde la liste des restaurations de configuration non encore acquittées. */
export function RecoveryProvider({ children }: { children: ReactNode }) {
  const [recoveries, setRecoveries] = useState<Recovery[]>([]);

  const report = useCallback((file: string, recovery: Recovery | null) => {
    setRecoveries((list) => {
      const others = list.filter((item) => item.file !== file);
      return recovery ? [...others, recovery] : others;
    });
  }, []);

  const acknowledge = useCallback(async (file: string) => {
    await ackRecovery(file);
    setRecoveries((list) => list.filter((item) => item.file !== file));
  }, []);

  const api = useMemo<RecoveryApi>(
    () => ({ recoveries, report, acknowledge }),
    [recoveries, report, acknowledge],
  );
  return <RecoveryContext.Provider value={api}>{children}</RecoveryContext.Provider>;
}
