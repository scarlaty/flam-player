import { useEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { libraryThumbnail } from "../../api/library";
import {
  listDeviceStories,
  type DeviceStories,
  type DeviceStory,
  type DeviceSummary,
  type IndexHealth,
} from "../../api/device";
import { toAppError, type AppError } from "../../api/error";
import { Badge, Banner, Button, EmptyState, ProgressBar } from "../../components";
import { t } from "../../i18n";
import { ErrorBanner } from "../ErrorBanner";
import { formatBytes } from "../validation";
import { useDeviceStatus } from "./useDeviceStatus";
import "./DeviceScreen.css";

const text = t.deviceScreen;

export interface DeviceScreenProps {
  /** Emplacement des actions globales (envoi d'une histoire, #230). */
  toolbar?: ReactNode;
  /** Emplacement des actions d'une histoire (retrait, #231). */
  storyActions?: (story: DeviceStory) => ReactNode;
}

/** Écran « Ma Flam » : état de la Flam branchée en USB et histoires installées (lecture seule). */
export function DeviceScreen({ toolbar, storyActions }: DeviceScreenProps) {
  const { status, error, retry } = useDeviceStatus();
  const device = status?.detected ? status.device : null;
  // Une nouvelle Flam (ou la même rebranchée) recharge la liste.
  const deviceKey = device ? `${device.root}\u0000${device.volume_label}` : null;

  const [announcement, setAnnouncement] = useState("");
  const wasDetected = useRef<boolean | null>(null);
  const retried = useRef(false);
  const headingRef = useRef<HTMLHeadingElement>(null);

  // Annonce les changements d'état (pas l'état initial « non détecté », déjà visible).
  const detected = device !== null;
  const known = status !== null;
  useEffect(() => {
    if (!known) return;
    const before = wasDetected.current;
    wasDetected.current = detected;
    if (before === detected || (before === null && !detected)) return;
    // Annonce d'un changement observé en dehors de toute interaction de l'utilisateur.
    setAnnouncement(detected ? text.announce.detected : text.announce.lost);
    if (detected && retried.current) headingRef.current?.focus();
    retried.current = false;
  }, [known, detected]);

  function onRetry() {
    retried.current = true;
    retry();
  }

  return (
    <div className="fl-device-screen">
      <p className="fl-device-screen__intro">{text.intro}</p>
      <p role="status" className="fl-visually-hidden">
        {announcement}
      </p>
      {error && (
        <ErrorBanner
          error={error}
          action={<Button onClick={onRetry}>{t.errors.retry}</Button>}
        />
      )}
      {!status && !error && <p>{text.searching}</p>}
      {status && !device && (
        <EmptyState
          icon="device"
          title={text.notDetected.title}
          description={text.notDetected.description}
          action={
            <Button variant="primary" icon="refresh" onClick={onRetry}>
              {text.notDetected.retry}
            </Button>
          }
        />
      )}
      {status && device && (
        <>
          <DeviceCard
            device={device}
            count={status.device_count}
            headingRef={headingRef}
          />
          {toolbar && <div className="fl-device-screen__toolbar">{toolbar}</div>}
          <DeviceStoriesList
            key={deviceKey}
            storyActions={storyActions}
            onCount={(count) => setAnnouncement(text.announce.stories(count))}
          />
        </>
      )}
    </div>
  );
}

function warningTexts(device: DeviceSummary): string[] {
  const messages = device.warnings.map((warning) => {
    switch (warning.kind) {
      case "missing_str_dir":
        return text.warnings.missingStr;
      case "missing_library_dir":
        return text.warnings.missingLibrary;
      case "unexpected_mdf_size":
        return text.warnings.unexpectedMdf(warning.size);
    }
  });
  if (device.mdf_problem?.kind === "truncated") messages.push(text.warnings.mdfTruncated);
  if (device.mdf_problem?.kind === "too_large") messages.push(text.warnings.mdfTooLarge);
  if (device.backup_failed) messages.push(text.warnings.backupFailed);
  return messages;
}

function DeviceCard({
  device,
  count,
  headingRef,
}: {
  device: DeviceSummary;
  count: number;
  headingRef: RefObject<HTMLHeadingElement | null>;
}) {
  const warnings = warningTexts(device);
  const space = device.space;
  const percent =
    space && space.total > 0 ? ((space.total - space.free) / space.total) * 100 : 0;
  return (
    <section className="fl-device-card" aria-labelledby="fl-device-title">
      <h2
        id="fl-device-title"
        ref={headingRef}
        tabIndex={-1}
        className="fl-device-card__title"
      >
        {text.detected.title}
      </h2>
      <dl className="fl-device-card__facts">
        <div>
          <dt>{text.detected.volume}</dt>
          <dd>
            {device.volume_label || text.detected.unnamedVolume} ({device.root})
          </dd>
        </div>
        {device.file_system && (
          <div>
            <dt>{text.detected.fileSystem}</dt>
            <dd>{device.file_system}</dd>
          </div>
        )}
      </dl>
      {space ? (
        <div className="fl-device-card__space">
          <ProgressBar value={percent} label={text.space.label} showValue />
          <p className="fl-device-card__space-text">
            {text.space.summary(formatBytes(space.free), formatBytes(space.total))}
          </p>
        </div>
      ) : (
        <p className="fl-device-card__space-text">{text.space.unavailable}</p>
      )}
      {count > 1 && (
        <Banner tone="info" role="status">
          {text.detected.several(count)}
        </Banner>
      )}
      {warnings.length > 0 && (
        <Banner tone="warning" title={text.warnings.title}>
          <ul className="fl-device-card__warnings">
            {warnings.map((message) => (
              <li key={message}>{message}</li>
            ))}
          </ul>
        </Banner>
      )}
    </section>
  );
}

type Notice = { tone: "warning" | "danger"; message: string };

const VISIBLE_NOTICES: Record<IndexHealth, string | null> = {
  valid: null,
  absent: text.index.visibleAbsent,
  damaged: text.index.visibleDamaged,
  unreadable: text.index.visibleUnreadable,
};

const HIDDEN_NOTICES: Record<IndexHealth, string | null> = {
  valid: null,
  absent: null,
  damaged: text.index.hiddenDamaged,
  unreadable: text.index.hiddenUnreadable,
};

function indexNotices(report: DeviceStories): Notice[] {
  const notices: Notice[] = [];
  const visible = VISIBLE_NOTICES[report.visible_index];
  if (visible) {
    notices.push({
      tone: report.visible_index === "unreadable" ? "danger" : "warning",
      message: visible,
    });
  }
  const hidden = HIDDEN_NOTICES[report.hidden_index];
  if (hidden) {
    notices.push({
      tone: report.hidden_index === "unreadable" ? "danger" : "warning",
      message: hidden,
    });
  }
  if (report.str_unreadable) {
    notices.push({ tone: "warning", message: text.index.strUnreadable });
  }
  return notices;
}

type StoriesState =
  | { phase: "loading" }
  | { phase: "error"; error: AppError }
  | { phase: "ready"; report: DeviceStories };

/** Liste des histoires de la Flam, chargée à l'affichage puis à la demande. */
function DeviceStoriesList({
  storyActions,
  onCount,
}: {
  storyActions?: (story: DeviceStory) => ReactNode;
  onCount: (count: number) => void;
}) {
  const [state, setState] = useState<StoriesState>({ phase: "loading" });
  const [reload, setReload] = useState(0);
  const wrapperRef = useRef<HTMLDivElement>(null);
  const onCountRef = useRef(onCount);
  useEffect(() => {
    onCountRef.current = onCount;
  });

  useEffect(() => {
    let active = true;
    listDeviceStories()
      .then((report) => {
        if (!active) return;
        if (!report) {
          // La Flam a disparu entre la détection et la lecture : jamais d'attente sans issue.
          setState({
            phase: "error",
            error: { kind: "not_found", message: text.stories.noAnswer },
          });
          return;
        }
        setState({ phase: "ready", report });
        onCountRef.current(report.stories.length);
      })
      .catch((caught) => {
        if (active) setState({ phase: "error", error: toAppError(caught) });
      });
    return () => {
      active = false;
    };
  }, [reload]);

  function refresh() {
    // Le bouton cliqué disparaît pendant le chargement : le focus passe à un élément stable.
    wrapperRef.current?.focus();
    setState({ phase: "loading" });
    setReload((n) => n + 1);
  }

  const renderStories = () => {
    if (state.phase === "error") {
      return (
        <ErrorBanner
          error={{
            ...state.error,
            message: `${text.stories.loadFailed} ${state.error.message}`,
          }}
          action={<Button onClick={refresh}>{t.errors.retry}</Button>}
        />
      );
    }
    if (state.phase === "loading") return <p>{text.stories.loading}</p>;

    const { report } = state;
    return (
      <section className="fl-device-stories" aria-labelledby="fl-device-stories-title">
        <div className="fl-device-stories__head">
          <h2 id="fl-device-stories-title" className="fl-device-card__title">
            {text.stories.title(report.stories.length)}
          </h2>
          <Button variant="secondary" size="sm" icon="refresh" onClick={refresh}>
            {text.stories.refresh}
          </Button>
        </div>
        {indexNotices(report).map(({ tone, message }) => (
          <Banner key={message} tone={tone} role="status">
            {message}
          </Banner>
        ))}
        {report.stories.length === 0 ? (
          <EmptyState
            icon="book"
            headingLevel={3}
            title={text.stories.emptyTitle}
            description={text.stories.emptyDescription}
          />
        ) : (
          <ul className="fl-device-rows" aria-label={text.stories.listLabel}>
            {report.stories.map((story) => (
              <DeviceStoryRow
                key={story.uuid}
                story={story}
                actions={storyActions?.(story)}
              />
            ))}
          </ul>
        )}
      </section>
    );
  };

  return (
    <div ref={wrapperRef} tabIndex={-1} className="fl-device-stories-wrap">
      {renderStories()}
    </div>
  );
}

/** Vignette conservée par la bibliothèque du launcher, si l'histoire y figure. */
function useStoryThumbnail(story: DeviceStory): string | undefined {
  const [thumb, setThumb] = useState<{ uuid: string; url: string }>();
  const { uuid, in_library: inLibrary } = story;
  useEffect(() => {
    if (!inLibrary) return;
    let active = true;
    libraryThumbnail(uuid)
      .then((url) => {
        if (active && url.startsWith("data:image/")) setThumb({ uuid, url });
      })
      .catch(() => undefined);
    return () => {
      active = false;
    };
  }, [uuid, inLibrary]);
  return thumb?.uuid === uuid ? thumb.url : undefined;
}

function DeviceStoryRow({ story, actions }: { story: DeviceStory; actions?: ReactNode }) {
  const thumb = useStoryThumbnail(story);
  const name = story.title ?? text.stories.untitled(story.short_id);
  return (
    <li className="fl-device-row">
      <span className="fl-device-row__thumb">
        {thumb ? <img src={thumb} alt="" /> : null}
      </span>
      <div className="fl-device-row__text">
        <span className="fl-device-row__name">{name}</span>
        {story.title && (
          <span className="fl-device-row__detail">
            {text.stories.idDetail(story.short_id)}
          </span>
        )}
      </div>
      {story.placement === "hidden" && (
        <Badge tone="secondary">{text.stories.hidden}</Badge>
      )}
      {story.placement === "unlisted" && (
        <Badge tone="warning" icon="alert">
          {text.stories.unlisted}
        </Badge>
      )}
      {story.missing && (
        <Badge tone="danger" icon="alert">
          {text.stories.missing}
        </Badge>
      )}
      {actions && <div className="fl-device-row__actions">{actions}</div>}
    </li>
  );
}
