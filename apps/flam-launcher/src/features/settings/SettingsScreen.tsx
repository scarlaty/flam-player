import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { getAppInfo, type AppInfo } from "../../api/appInfo";
import { clearCache, getCacheUsage, type CacheUsage } from "../../api/cache";
import { pickExecutable, pickFolder } from "../../api/dialog";
import { toAppError, type AppError } from "../../api/error";
import {
  getSettings,
  setSettings,
  type SettingsIssue,
  type SettingsView,
} from "../../api/settings";
import {
  Banner,
  Button,
  CheckboxField,
  Dialog,
  RadioGroup,
  TextField,
  useToast,
} from "../../components";
import { t } from "../../i18n";
import { applyTheme, THEME_PREFERENCES, type ThemePreference } from "../../theme/theme";
import { ErrorBanner } from "../ErrorBanner";
import { getMinimizeDuringPlay, setMinimizeDuringPlay } from "./preferences";
import { useRecovery } from "../recovery/RecoveryContext";
import { formatBytes, validateOptionalPath } from "../validation";
import "./SettingsScreen.css";

type PathField = "player_path" | "python_path" | "cache_dir";

/** Écran des paramètres : chemins, cache, thème et à propos. */
export function SettingsScreen() {
  const toast = useToast();
  const { report } = useRecovery();
  const [view, setView] = useState<SettingsView | null>(null);
  const [loadError, setLoadError] = useState<AppError | null>(null);
  const [saveError, setSaveError] = useState<AppError | null>(null);
  const [values, setValues] = useState<Record<PathField, string>>({
    player_path: "",
    python_path: "",
    cache_dir: "",
  });
  const [touched, setTouched] = useState<Record<PathField, boolean>>({
    player_path: false,
    python_path: false,
    cache_dir: false,
  });
  const [usage, setUsage] = useState<CacheUsage | null>(null);
  const [usageError, setUsageError] = useState<AppError | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [themeError, setThemeError] = useState(false);
  const [minimizeDuringPlay, setMinimize] = useState(getMinimizeDuringPlay);
  const cancelClearRef = useRef<HTMLButtonElement>(null);
  // Derniers paramètres confirmés par le backend et numéro du dernier changement de thème.
  const confirmedRef = useRef<SettingsView | null>(null);
  const themeRequestRef = useRef(0);
  // Numéro de la requête dont la réponse est dans `confirmedRef`, et requêtes en vol.
  const confirmedRequestRef = useRef(0);
  const themeInFlightRef = useRef(0);

  const refreshUsage = useCallback(async () => {
    try {
      setUsage(await getCacheUsage());
      setUsageError(null);
    } catch (caught) {
      setUsageError(toAppError(caught));
    }
  }, []);

  const applyView = useCallback(
    (next: SettingsView) => {
      confirmedRef.current = next;
      setView(next);
      setValues({
        player_path: next.settings.player_path ?? "",
        python_path: next.settings.python_path ?? "",
        cache_dir: next.settings.cache_dir ?? "",
      });
      setTouched({ player_path: false, python_path: false, cache_dir: false });
      report("settings.json", next.recovery);
    },
    [report],
  );

  const load = useCallback(async () => {
    setLoadError(null);
    try {
      applyView(await getSettings());
    } catch (caught) {
      setLoadError(toAppError(caught));
    }
  }, [applyView]);

  useEffect(() => {
    // Chargement initial depuis le backend.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void load();
    void refreshUsage();
    getAppInfo()
      .then(setInfo)
      .catch(() => setInfo(null));
  }, [load, refreshUsage]);

  const errors: Record<PathField, string | null> = {
    player_path: validateOptionalPath(values.player_path, { exe: true }),
    python_path: validateOptionalPath(values.python_path, { exe: true }),
    cache_dir: validateOptionalPath(values.cache_dir, { exe: false }),
  };
  const shown = (field: PathField) => (touched[field] ? errors[field] : null);

  function edit(field: PathField, value: string) {
    setValues((current) => ({ ...current, [field]: value }));
    setTouched((current) => ({ ...current, [field]: true }));
  }

  async function browse(field: PathField) {
    try {
      const path =
        field === "cache_dir"
          ? await pickFolder(t.settingsScreen.pickCache)
          : await pickExecutable(
              field === "player_path"
                ? t.settingsScreen.pickPlayer
                : t.settingsScreen.pickPython,
            );
      if (path) edit(field, path);
    } catch (caught) {
      setSaveError(toAppError(caught));
    }
  }

  async function save(event: FormEvent) {
    event.preventDefault();
    setTouched({ player_path: true, python_path: true, cache_dir: true });
    if (!view || Object.values(errors).some(Boolean)) return;
    setSaveError(null);
    try {
      const next = await setSettings({
        ...view.settings,
        player_path: values.player_path.trim() || null,
        python_path: values.python_path.trim() || null,
        cache_dir: values.cache_dir.trim() || null,
      });
      applyView(next);
      void refreshUsage();
      toast.show({ message: t.settingsScreen.saved, tone: "success" });
    } catch (caught) {
      setSaveError(toAppError(caught));
    }
  }

  async function changeTheme(theme: ThemePreference) {
    const confirmed = confirmedRef.current;
    if (!view || !confirmed) return;
    const request = ++themeRequestRef.current;
    const isLatest = () => request === themeRequestRef.current;
    const showConfirmedTheme = () => {
      const settled = confirmedRef.current ?? confirmed;
      setView((current) =>
        current
          ? {
              ...current,
              settings: { ...current.settings, theme: settled.settings.theme },
            }
          : current,
      );
      applyTheme(settled.settings.theme);
    };
    themeInFlightRef.current += 1;
    setThemeError(false);
    setView((current) =>
      current ? { ...current, settings: { ...current.settings, theme } } : current,
    );
    applyTheme(theme);
    try {
      const next = await setSettings({ ...confirmed.settings, theme });
      // Une réponse plus ancienne n'écrase pas un état confirmé par une requête plus récente.
      if (request > confirmedRequestRef.current) {
        confirmedRef.current = next;
        confirmedRequestRef.current = request;
      }
      if (isLatest()) setView(next);
    } catch {
      if (isLatest()) {
        // Retour au dernier état réellement enregistré, pas à celui capturé à l'appel.
        showConfirmedTheme();
        setThemeError(true);
      }
    } finally {
      themeInFlightRef.current -= 1;
      // Plus rien en vol : l'affichage doit refléter exactement le thème enregistré.
      if (themeInFlightRef.current === 0) showConfirmedTheme();
    }
  }

  async function confirmClearCache() {
    setConfirmClear(false);
    try {
      const { freed_bytes } = await clearCache();
      toast.show({ message: t.settingsScreen.cache.cleared(formatBytes(freed_bytes)) });
      await refreshUsage();
    } catch (caught) {
      setUsageError(toAppError(caught));
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
  if (!view) return <p>{t.common.loading}</p>;

  const text = t.settingsScreen;
  const hintFor = (field: PathField) => {
    const used = view.resolved[field];
    return used ? text.used(used) : text.notFound;
  };
  const issues: SettingsIssue[] = view.issues;

  return (
    <div className="fl-settings">
      <section className="fl-settings__section" aria-labelledby="settings-paths">
        <h2 id="settings-paths">{text.paths}</h2>
        <p>{text.pythonNote}</p>
        {issues.length > 0 && (
          <Banner tone="warning" title={text.issuesTitle}>
            {issues.map((issue) => (
              <p key={`${issue.field}-${issue.kind}`}>{issue.message}</p>
            ))}
          </Banner>
        )}
        {saveError && <ErrorBanner error={saveError} />}
        <form className="fl-form" noValidate onSubmit={save}>
          {(
            [
              ["player_path", text.player],
              ["python_path", text.python],
              ["cache_dir", text.cacheDir],
            ] as const
          ).map(([field, label]) => (
            <TextField
              key={field}
              label={label}
              value={values[field]}
              placeholder={text.autoPlaceholder}
              hint={hintFor(field)}
              error={shown(field)}
              onChange={(e) => edit(field, e.target.value)}
              actions={
                <>
                  <Button size="sm" onClick={() => void browse(field)}>
                    {text.browse}
                  </Button>
                  <Button size="sm" variant="ghost" onClick={() => edit(field, "")}>
                    {text.automatic}
                  </Button>
                </>
              }
            />
          ))}
          <div>
            <Button variant="primary" type="submit">
              {text.save}
            </Button>
          </div>
        </form>
      </section>

      <section className="fl-settings__section" aria-labelledby="settings-cache">
        <h2 id="settings-cache">{text.cache.title}</h2>
        {usageError && <ErrorBanner error={usageError} />}
        {usage && (
          <p>
            {usage.bytes > 0
              ? text.cache.usage(formatBytes(usage.bytes), usage.path)
              : text.cache.empty(usage.path)}
          </p>
        )}
        <div>
          <Button
            variant="danger"
            icon="trash"
            disabled={!usage || usage.bytes === 0}
            onClick={() => setConfirmClear(true)}
          >
            {text.cache.clear}
          </Button>
        </div>
      </section>

      <section className="fl-settings__section" aria-labelledby="settings-appearance">
        <h2 id="settings-appearance">{text.appearance}</h2>
        {themeError && <Banner tone="danger">{text.themeFailed}</Banner>}
        <RadioGroup
          legend={t.theme.label}
          value={view.settings.theme}
          options={THEME_PREFERENCES.map((value) => ({ value, label: t.theme[value] }))}
          onChange={(theme) => void changeTheme(theme)}
        />
      </section>

      <section className="fl-settings__section" aria-labelledby="settings-playback">
        <h2 id="settings-playback">{text.playback.title}</h2>
        <CheckboxField
          label={text.playback.minimize}
          hint={text.playback.minimizeHint}
          checked={minimizeDuringPlay}
          onChange={(event) => {
            setMinimize(event.target.checked);
            setMinimizeDuringPlay(event.target.checked);
          }}
        />
      </section>

      <section className="fl-settings__section" aria-labelledby="settings-about">
        <h2 id="settings-about">{text.about.title}</h2>
        {info && <p>{text.about.version(info.name, info.version)}</p>}
        <p>{text.about.license}</p>
        <p>{text.about.credits}</p>
      </section>

      <Dialog
        open={confirmClear}
        onClose={() => setConfirmClear(false)}
        role="alertdialog"
        title={text.cache.confirmTitle}
        description={
          usage
            ? text.cache.confirmDescription(
                usage.subdirs.join(", "),
                usage.path,
                formatBytes(usage.bytes),
              )
            : ""
        }
        initialFocus={cancelClearRef}
        footer={
          <>
            <Button ref={cancelClearRef} onClick={() => setConfirmClear(false)}>
              {t.common.cancel}
            </Button>
            <Button variant="danger" onClick={() => void confirmClearCache()}>
              {text.cache.clear}
            </Button>
          </>
        }
      />
    </div>
  );
}
