import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { getAppInfo, type AppInfo } from "../../api/appInfo";
import {
  clearCache,
  getCacheUsage,
  getMp3CacheUsage,
  type CacheUsage,
} from "../../api/cache";
import { pickExecutable, pickFolder } from "../../api/dialog";
import { toAppError, type AppError } from "../../api/error";
import {
  getSettings,
  THUMB_FITS,
  type Settings,
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
import { useSettingsSaver } from "./useSettingsSaver";
import { VoiceSection } from "./VoiceSection";
import "./SettingsScreen.css";

type PathField = "player_path" | "cache_dir";

/** Écran des paramètres : chemins, cache, thème et à propos. */
export function SettingsScreen() {
  const toast = useToast();
  const { report } = useRecovery();
  const saver = useSettingsSaver();
  const { view, adopt } = saver;
  const [loadError, setLoadError] = useState<AppError | null>(null);
  const [saveError, setSaveError] = useState<AppError | null>(null);
  const [values, setValues] = useState<Record<PathField, string>>({
    player_path: "",
    cache_dir: "",
  });
  const [touched, setTouched] = useState<Record<PathField, boolean>>({
    player_path: false,
    cache_dir: false,
  });
  const [usage, setUsage] = useState<CacheUsage | null>(null);
  const [usageError, setUsageError] = useState<AppError | null>(null);
  // Part des MP3 de podcasts ; `null` : pas encore connue ou illisible (la ligne est alors omise).
  const [mp3Bytes, setMp3Bytes] = useState<number | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [themeError, setThemeError] = useState(false);
  const [thumbFitError, setThumbFitError] = useState(false);
  const [voiceError, setVoiceError] = useState(false);
  const [minimizeDuringPlay, setMinimize] = useState(getMinimizeDuringPlay);
  const cancelClearRef = useRef<HTMLButtonElement>(null);
  // Numéro de la dernière modification demandée, par réglage.
  const requestsRef = useRef<Record<string, number>>({});
  // Enregistrement des chemins en cours (un second envoi du formulaire est ignoré).
  const pathsSavingRef = useRef(false);

  const refreshUsage = useCallback(async () => {
    try {
      setUsage(await getCacheUsage());
      setUsageError(null);
    } catch (caught) {
      setUsageError(toAppError(caught));
    }
    try {
      setMp3Bytes((await getMp3CacheUsage()).bytes);
    } catch {
      // Information secondaire : un échec n'empêche ni l'affichage ni le vidage du cache.
      setMp3Bytes(null);
    }
  }, []);

  const applyView = useCallback(
    (next: SettingsView) => {
      adopt(next);
      setValues({
        player_path: next.settings.player_path ?? "",
        cache_dir: next.settings.cache_dir ?? "",
      });
      setTouched({ player_path: false, cache_dir: false });
      report("settings.json", next.recovery);
    },
    [adopt, report],
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
          : await pickExecutable(t.settingsScreen.pickPlayer);
      if (path) edit(field, path);
    } catch (caught) {
      setSaveError(toAppError(caught));
    }
  }

  async function save(event: FormEvent) {
    event.preventDefault();
    setTouched({ player_path: true, cache_dir: true });
    if (!view || pathsSavingRef.current || Object.values(errors).some(Boolean)) return;
    pathsSavingRef.current = true;
    setSaveError(null);
    try {
      // Seuls les chemins sont envoyés : le reste vient de l'état confirmé, pas de l'affichage
      // (un thème ou une voix en cours d'enregistrement n'est ni écrasé ni anticipé).
      const next = await saver.save(
        {
          player_path: values.player_path.trim() || null,
          cache_dir: values.cache_dir.trim() || null,
        },
        false,
      );
      applyView(next);
      void refreshUsage();
      toast.show({ message: t.settingsScreen.saved, tone: "success" });
    } catch (caught) {
      setSaveError(toAppError(caught));
    } finally {
      pathsSavingRef.current = false;
    }
  }

  /**
   * Enregistre un réglage (thème, vignettes, voix) : affiché tout de suite, puis enregistré à la
   * suite des autres modifications (file du mécanisme commun). Seul le dernier choix d'un même
   * réglage signale un échec ; l'affichage revient alors à l'état enregistré.
   */
  async function change<K extends "theme" | "thumb_fit" | "tts_voice">(
    key: K,
    value: Settings[K],
    setError: (failed: boolean) => void,
  ) {
    if (!saver.current()) return;
    const request = (requestsRef.current[key] = (requestsRef.current[key] ?? 0) + 1);
    setError(false);
    if (key === "theme") applyTheme(value as ThemePreference);
    try {
      await saver.save({ [key]: value }, true);
    } catch {
      if (requestsRef.current[key] === request) setError(true);
    } finally {
      // L'interface reflète le thème affiché (choix en attente, sinon thème enregistré).
      const shown = saver.current();
      if (shown) applyTheme(shown.settings.theme);
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
        {usage && mp3Bytes !== null && (
          <p>
            {mp3Bytes > 0 ? text.cache.mp3(formatBytes(mp3Bytes)) : text.cache.mp3Empty}
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
          onChange={(theme) => void change("theme", theme, setThemeError)}
        />
      </section>

      <section className="fl-settings__section" aria-labelledby="settings-thumbs">
        <h2 id="settings-thumbs">{text.thumbFit.title}</h2>
        <p>{text.thumbFit.note}</p>
        {thumbFitError && <Banner tone="danger">{text.thumbFit.failed}</Banner>}
        <RadioGroup
          legend={text.thumbFit.label}
          value={view.settings.thumb_fit}
          options={THUMB_FITS.map((value) => ({
            value,
            label: text.thumbFit.options[value].label,
            hint: text.thumbFit.options[value].hint,
          }))}
          onChange={(thumbFit) => void change("thumb_fit", thumbFit, setThumbFitError)}
        />
      </section>

      <VoiceSection
        value={view.settings.tts_voice ?? null}
        failed={voiceError}
        onChange={(voice) => void change("tts_voice", voice, setVoiceError)}
      />

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
