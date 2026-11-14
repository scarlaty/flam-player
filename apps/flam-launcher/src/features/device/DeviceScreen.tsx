import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import {
  firmwareWriteBlock,
  type DeviceStories,
  type DeviceStory,
  type DeviceSummary,
  type IndexHealth,
} from "../../api/device";
import { Badge, Banner, Button, EmptyState, Icon, ProgressBar } from "../../components";
import { t } from "../../i18n";
import { ErrorBanner } from "../ErrorBanner";
import { formatBytes } from "../validation";
import { useDevice } from "./DeviceContext";
import { DeviceStoryOrder } from "./DeviceStoryOrder";
import { DiagnosticPanel } from "./DiagnosticPanel";
import { EjectButton } from "./EjectButton";
import { ExperimentalBanner } from "./ExperimentalBanner";
import { FirmwareNotice, firmwareSummary } from "./FirmwareStatus";
import { ForgetCarrierKeysButton } from "./ForgetCarrierKeysButton";
import { RemoveStoryButton } from "./RemoveStoryButton";
import {
  ThumbnailLoader,
  sentStamps,
  useDeviceStoryTitles,
  useDeviceThumbnail,
  useOfficialStoryTitles,
  useOfficialThumbnail,
  type OfficialInfo,
} from "./storyDetails";
import "./DeviceScreen.css";

const text = t.deviceScreen;

export interface DeviceScreenProps {
  /** Section affichée sous la liste des histoires de la Flam (envoi depuis « Mes histoires »). */
  libraryPanel?: ReactNode;
}

/**
 * Écran « Ma Flam » : état de la Flam branchée en USB, ses histoires, envoi, retrait et éjection,
 * diagnostic copiable pour les retours de test.
 */
export function DeviceScreen({ libraryPanel }: DeviceScreenProps) {
  const {
    status,
    error,
    retry,
    retryAnswered,
    device,
    deviceKey,
    stories,
    recoveryNote,
    ejectNote,
  } = useDevice();
  const count =
    device && stories.phase === "ready" ? stories.report.stories.length : null;

  // Annonces : l'état de la Flam puis le nombre d'histoires, dites ensemble pour que la
  // seconde ne couvre pas la première.
  const [stateNote, setStateNote] = useState("");
  const countNote = count === null ? "" : text.announce.stories(count);
  const announcement = [stateNote, countNote].filter(Boolean).join(". ");
  const wasDetected = useRef<boolean | null>(null);
  // Numéro de la demande « Réessayer » dont le résultat doit recevoir le focus (0 : aucune).
  const pendingRetry = useRef(0);
  const headingRef = useRef<HTMLHeadingElement>(null);

  // Annonce les changements d'état (pas l'état initial « non détecté », déjà visible).
  const detected = device !== null;
  const known = status !== null;
  useEffect(() => {
    if (!known) return;
    const before = wasDetected.current;
    wasDetected.current = detected;
    if (before === detected || (before === null && !detected)) return;
    setStateNote(detected ? text.announce.detected : text.announce.lost);
    if (detected && pendingRetry.current > 0) headingRef.current?.focus();
  }, [known, detected]);

  // « Réessayer » ne réclame le focus que pour la réponse d'un appel lancé après le clic : celle
  // d'un appel déjà en vol (numéro plus ancien) ne l'éteint pas.
  useEffect(() => {
    if (retryAnswered >= pendingRetry.current) pendingRetry.current = 0;
  }, [retryAnswered]);

  function onRetry() {
    pendingRetry.current = retry();
  }

  return (
    <div className="fl-device-screen">
      <p className="fl-device-screen__intro">{text.intro}</p>
      <ExperimentalBanner family={device?.firmware_status?.family ?? null} />
      <p role="status" className="fl-visually-hidden">
        {announcement}
      </p>
      {ejectNote && (
        <Banner
          tone={ejectNote.tone}
          role={ejectNote.tone === "danger" ? "alert" : "status"}
        >
          {ejectNote.message}
        </Banner>
      )}
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
          <FirmwareNotice device={device} />
          {recoveryNote && (
            <Banner tone={recoveryNote.tone} role="status">
              {recoveryNote.message}
            </Banner>
          )}
          <div className="fl-device-screen__toolbar">
            <EjectButton />
            <ForgetCarrierKeysButton />
          </div>
          {/* Autre Flam, même sous la même lettre (#609) : la liste et ses dialogues repartent
              de zéro. */}
          <DeviceStoriesList key={deviceKey ?? ""} />
          {libraryPanel}
        </>
      )}
      {/* Toujours affiché : sans Flam, ou écriture bloquée, c'est là que le diagnostic sert. */}
      <DiagnosticPanel />
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
        <div>
          <dt>{text.detected.firmware}</dt>
          <dd>
            {device.firmware_status
              ? firmwareSummary(device.firmware_status)
              : (device.firmware ?? text.detected.firmwareUnreadable)}
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
  if (report.truncated) {
    notices.push({ tone: "warning", message: text.index.truncated(report.limit) });
  }
  if (report.str_unreadable) {
    notices.push({ tone: "warning", message: text.index.strUnreadable });
  }
  return notices;
}

/** Ordre modifiable : index sûrs et liste complète d'au moins deux histoires visibles. */
function canReorder(report: DeviceStories | null): report is DeviceStories {
  return (
    report !== null &&
    report.visible_index === "valid" &&
    (report.hidden_index === "valid" || report.hidden_index === "absent") &&
    !report.truncated &&
    report.stories.filter((s) => s.placement === "visible").length >= 2
  );
}

/**
 * Liste des histoires de la Flam (lues par le `DeviceProvider`), avec retrait par histoire et
 * modification de l'ordre.
 */
function DeviceStoriesList() {
  const { stories: state, reloadStories, busy, device, queue } = useDevice();
  const root = device?.root ?? null;
  const report = state.phase === "ready" ? state.report : null;
  const readyUuids = useMemo(
    () => report?.stories.filter((s) => !s.missing).map((s) => s.uuid) ?? [],
    [report],
  );
  // Relecture de la liste (phase `loading`) : la liste d'UUID ne passe pas par un état vide,
  // qui ferait oublier les titres déjà lus (clignotement). Elle est conservée tant que la
  // même Flam est branchée ; une liste prête (même vide) la remplace.
  const [lastUuids, setLastUuids] = useState<{ root: string | null; uuids: string[] }>({
    root,
    uuids: readyUuids,
  });
  const loading = state.phase === "loading";
  if (state.phase === "ready" && lastUuids.uuids !== readyUuids)
    setLastUuids({ root, uuids: readyUuids });
  if (lastUuids.root !== root) setLastUuids({ root, uuids: readyUuids });
  const uuids = loading && lastUuids.root === root ? lastUuids.uuids : readyUuids;
  // Titres et vignettes de la Flam : chargés après l'affichage de la liste, suspendus pendant
  // un envoi ou un retrait.
  // Un envoi terminé pour un UUID (« Remplacer sur la Flam » le garde) : titre et vignette lus
  // avant l'envoi sont oubliés et relus.
  const sent = useMemo(() => sentStamps(queue), [queue]);
  // « Actualiser » demandé (pour la Flam `root`) : à l'arrivée de la liste relue, même identique,
  // le compteur avance et relance la lecture des titres restés sans réponse (clés illisibles),
  // sinon le bandeau resterait en place. Les rechargements silencieux (après un envoi ou un
  // retrait) n'y touchent pas : à liste identique, `device_story_titles` n'est pas rappelé (une
  // liste qui change relance la lecture de toute façon). La demande est abandonnée si la
  // relecture échoue ou si la Flam change, pour ne pas servir au rapport suivant.
  const [rereadAsked, setRereadAsked] = useState<{ root: string | null } | null>(null);
  const [reads, setReads] = useState<{ report: DeviceStories | null; count: number }>({
    report: null,
    count: 0,
  });
  const askedHere = rereadAsked?.root === root && state.phase !== "error";
  if (report && reads.report !== report) {
    setReads({ report, count: reads.count + (askedHere ? 1 : 0) });
    if (rereadAsked) setRereadAsked(null);
  } else if (rereadAsked && !askedHere) {
    setRereadAsked(null);
  }
  const { titles, lunii, keysProblem } = useDeviceStoryTitles(
    root,
    uuids,
    busy,
    sent,
    reads.count,
  );
  // Titre non déchiffrable (clé d'un autre compte) : catalogue officiel en ligne, seulement s'il
  // reste au moins une histoire sans titre.
  const official = useOfficialStoryTitles(titles, lunii);
  const loader = useMemo(() => (root ? new ThumbnailLoader(root) : null), [root]);
  // Vignette lue avant un envoi terminé : oubliée avant le rendu des lignes (leur clé change
  // avec l'envoi), qui la redemandent. Idempotent : un second rendu ne fait rien de plus.
  loader?.forgetSent(sent);
  useEffect(() => () => loader?.dispose(), [loader]);
  useEffect(() => loader?.setPaused(busy), [loader, busy]);
  const wrapperRef = useRef<HTMLDivElement>(null);
  const orderButtonRef = useRef<HTMLButtonElement>(null);
  const [ordering, setOrdering] = useState(false);
  const reorderable = canReorder(report);
  // Liste relue non modifiable (index abîmé, tronquée…) : l'édition de l'ordre se ferme.
  if (ordering && !reorderable) setOrdering(false);
  const showOrder = ordering && reorderable;
  const orderHintId = useId();
  // Après un retrait, la ligne (donc le bouton) disparaît à la relecture : le focus passe à la
  // liste dès que la nouvelle liste est affichée.
  const focusAfterReload = useRef(false);
  const [removing, setRemoving] = useState<string | null>(null);
  useEffect(() => {
    if (focusAfterReload.current && state.phase === "ready") {
      focusAfterReload.current = false;
      wrapperRef.current?.focus();
    }
  }, [state]);

  // Fin de l'édition de l'ordre : le focus revient au bouton « Modifier l'ordre » réaffiché.
  const focusOrderButton = useRef(false);
  useEffect(() => {
    if (showOrder || !focusOrderButton.current) return;
    focusOrderButton.current = false;
    (orderButtonRef.current ?? wrapperRef.current)?.focus();
  }, [showOrder]);

  function closeOrder() {
    focusOrderButton.current = true;
    setOrdering(false);
  }

  // Même nom que la ligne : titre déchiffré, catalogue officiel (sauf Lunii), bibliothèque, UUID court.
  const nameOf = (story: DeviceStory) =>
    storyName(
      story,
      titles.get(story.uuid),
      lunii.has(story.uuid) ? undefined : official.get(story.uuid),
    );

  function refresh() {
    // Le bouton cliqué disparaît pendant le chargement : le focus passe à un élément stable.
    wrapperRef.current?.focus();
    setRereadAsked({ root });
    reloadStories();
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
    const removingStory = removing
      ? report.stories.find((story) => story.uuid === removing)
      : undefined;
    return (
      <section className="fl-device-stories" aria-labelledby="fl-device-stories-title">
        <div className="fl-device-stories__head">
          <h2 id="fl-device-stories-title" className="fl-device-card__title">
            {text.stories.title(report.stories.length)}
          </h2>
          {!showOrder && (
            <div className="fl-device-row__actions">
              {reorderable && (
                <span className="fl-send">
                  <Button
                    ref={orderButtonRef}
                    variant="secondary"
                    size="sm"
                    icon="sliders"
                    className="fl-send__button"
                    aria-disabled={busy || undefined}
                    aria-describedby={busy ? orderHintId : undefined}
                    onClick={() => {
                      if (!busy) setOrdering(true);
                    }}
                  >
                    {text.order.edit}
                  </Button>
                  {busy && (
                    <span id={orderHintId} className="fl-send__reason">
                      {text.order.busy}
                    </span>
                  )}
                </span>
              )}
              <Button variant="secondary" size="sm" icon="refresh" onClick={refresh}>
                {text.stories.refresh}
              </Button>
            </div>
          )}
        </div>
        {removingStory && (
          <p role="status" className="fl-visually-hidden">
            {t.deviceTransfer.remove.workingNamed(nameOf(removingStory))}
          </p>
        )}
        {keysProblem && firmwareWriteBlock(device)?.kind !== "keys_unreadable" && (
          // Doublon évité seulement quand le bandeau du firmware donne déjà cette raison
          // (clés illisibles) ; un autre blocage d'écriture ne dit rien des titres.
          <Banner tone="warning" role="status" title={text.stories.keysProblemTitle}>
            {text.stories.keysProblem(keysProblem)}
          </Banner>
        )}
        {indexNotices(report).map(({ tone, message }) => (
          <Banner key={message} tone={tone} role="status">
            {message}
          </Banner>
        ))}
        {showOrder ? (
          <DeviceStoryOrder
            stories={report.stories.filter((s) => s.placement === "visible")}
            nameOf={nameOf}
            hasOthers={report.stories.some((s) => s.placement !== "visible")}
            busy={busy}
            onClose={closeOrder}
          />
        ) : report.stories.length === 0 ? (
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
                key={`${story.uuid}:${sent.get(story.uuid) ?? 0}`}
                story={story}
                name={nameOf(story)}
                deviceTitle={titles.get(story.uuid)}
                lunii={lunii.has(story.uuid)}
                official={lunii.has(story.uuid) ? undefined : official.get(story.uuid)}
                loader={story.missing ? null : loader}
                removing={removing === story.uuid}
                actions={
                  <RemoveStoryButton
                    story={story}
                    name={nameOf(story)}
                    busy={busy}
                    locked={removing !== null && removing !== story.uuid}
                    onWorkingChange={(working) =>
                      setRemoving((current) =>
                        working ? story.uuid : current === story.uuid ? null : current,
                      )
                    }
                    onRemoved={() => {
                      focusAfterReload.current = true;
                    }}
                  />
                }
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

/**
 * Nom d'une histoire de la Flam : titre déchiffré (`deviceTitle`), sinon celui du catalogue
 * officiel (`official`), sinon celui de la bibliothèque, sinon l'UUID court. Commun à la ligne,
 * au bouton « Retirer » et à l'édition de l'ordre.
 */
function storyName(
  story: DeviceStory,
  deviceTitle: string | null | undefined,
  official: OfficialInfo | undefined,
): string {
  return (
    deviceTitle ?? official?.title ?? story.title ?? text.stories.untitled(story.short_id)
  );
}

/**
 * Ligne d'une histoire. Titre : celui déchiffré sur la Flam (`deviceTitle`), sinon celui du
 * catalogue officiel (`official`), sinon celui de la bibliothèque, sinon l'UUID court. Vignette :
 * celle de la Flam, sinon l'image du catalogue, sinon l'illustration par défaut.
 */
function DeviceStoryRow({
  story,
  name,
  deviceTitle,
  lunii = false,
  official,
  loader,
  actions,
  removing = false,
}: {
  story: DeviceStory;
  /** Nom affiché (`storyName`), repris par le bouton de retrait. */
  name: string;
  deviceTitle?: string | null;
  /** Histoire au format Lunii/STUdio (badge discret). */
  lunii?: boolean;
  official?: OfficialInfo;
  loader: ThumbnailLoader | null;
  actions?: ReactNode;
  /** Retrait en cours : la ligne est grisée. */
  removing?: boolean;
}) {
  const rowRef = useRef<HTMLLIElement>(null);
  const deviceThumb = useDeviceThumbnail(loader, story.uuid, rowRef);
  const catalogThumb = useOfficialThumbnail(
    story.uuid,
    deviceThumb === null && official?.hasImage === true,
  );
  const thumb = deviceThumb ?? catalogThumb;
  const title = deviceTitle ?? official?.title ?? story.title;
  return (
    <li ref={rowRef} className="fl-device-row" aria-busy={removing || undefined}>
      <span className="fl-device-row__thumb">
        {thumb ? <img src={thumb} alt="" /> : <Icon name="book" size={20} />}
      </span>
      <div className="fl-device-row__text">
        <span className="fl-device-row__name">{name}</span>
        {title && (
          <span className="fl-device-row__detail">
            {text.stories.idDetail(story.short_id)}
          </span>
        )}
      </div>
      {lunii && <Badge tone="secondary">{text.stories.lunii}</Badge>}
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
