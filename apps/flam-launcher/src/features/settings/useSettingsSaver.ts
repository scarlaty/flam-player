import { useCallback, useRef, useState } from "react";
import { setSettings, type Settings, type SettingsView } from "../../api/settings";

/** Modification d'un ou plusieurs champs des paramètres. */
export type SettingsPatch = Partial<Settings>;

export interface SettingsSaver {
  /** Paramètres affichés : derniers paramètres confirmés, avec les choix en cours d'enregistrement. */
  view: SettingsView | null;
  /** Même valeur que `view`, lue à l'instant (hors rendu : fin d'une requête). */
  current: () => SettingsView | null;
  /** Adopte des paramètres lus du backend (`settings_get`) comme état confirmé. */
  adopt: (next: SettingsView) => void;
  /**
   * Enregistre `patch` par-dessus les derniers paramètres confirmés. `settings_set` remplace
   * l'objet entier : les enregistrements sont donc exécutés l'un après l'autre (file), chacun
   * partant de l'état confirmé par le précédent, jamais d'une copie lue plus tôt. Avec
   * `optimistic`, le choix est affiché tout de suite puis retiré à la fin de la requête
   * (succès : il est alors dans l'état confirmé ; échec : l'affichage revient à l'état confirmé).
   * Rejette si l'enregistrement échoue ; la file continue.
   */
  save: (patch: SettingsPatch, optimistic: boolean) => Promise<SettingsView>;
}

interface Pending {
  id: number;
  patch: SettingsPatch;
}

/** Mécanisme commun d'enregistrement des paramètres (chemins, thème, vignettes, voix). */
export function useSettingsSaver(): SettingsSaver {
  const [view, setView] = useState<SettingsView | null>(null);
  const confirmed = useRef<SettingsView | null>(null);
  const pending = useRef<Pending[]>([]);
  const tail = useRef<Promise<unknown>>(Promise.resolve());
  const sequence = useRef(0);

  const compute = useCallback((): SettingsView | null => {
    const base = confirmed.current;
    if (!base) return null;
    if (pending.current.length === 0) return base;
    const settings = pending.current.reduce<Settings>(
      (acc, entry) => ({ ...acc, ...entry.patch }),
      base.settings,
    );
    return { ...base, settings };
  }, []);

  const publish = useCallback(() => setView(compute()), [compute]);

  const adopt = useCallback(
    (next: SettingsView) => {
      confirmed.current = next;
      publish();
    },
    [publish],
  );

  const save = useCallback(
    (patch: SettingsPatch, optimistic: boolean): Promise<SettingsView> => {
      const entry: Pending = { id: ++sequence.current, patch };
      if (optimistic) {
        pending.current.push(entry);
        publish();
      }
      const run = async (): Promise<SettingsView> => {
        try {
          const base = confirmed.current;
          if (!base) throw new Error("paramètres non chargés");
          const next = await setSettings({ ...base.settings, ...patch });
          confirmed.current = next;
          return next;
        } finally {
          pending.current = pending.current.filter((other) => other.id !== entry.id);
          publish();
        }
      };
      const result = tail.current.then(run, run);
      tail.current = result.catch(() => undefined);
      return result;
    },
    [publish],
  );

  return { view, current: compute, adopt, save };
}
