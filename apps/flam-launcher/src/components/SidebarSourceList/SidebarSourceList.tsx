import {
  useId,
  useRef,
  type KeyboardEvent,
  type ReactNode,
  type SyntheticEvent,
} from "react";
import { t } from "../../i18n";
import { Button } from "../Button/Button";
import { cx } from "../cx";
import { Icon, type IconName } from "../Icon";
import "./SidebarSourceList.css";

export type SourceKind = "library" | "catalog" | "folder";

/** `offline` : source injoignable pour l'instant ; `error` : source illisible ou invalide. */
export type SourceStatus = "ok" | "offline" | "error";

export interface SidebarSource {
  id: string;
  label: string;
  kind: SourceKind;
  /** Nombre d'histoires, si connu (affiché seulement quand la source est disponible). */
  count?: number;
  status?: SourceStatus;
}

export interface SidebarSourceListProps {
  /** Sources, affichées dans l'ordre fourni. */
  sources: SidebarSource[];
  selectedId?: string;
  onSelect: (id: string) => void;
  /** Titre de la liste (« Sources » par défaut). */
  title?: string;
  /**
   * Actions propres à une source (renommer, supprimer, réordonner…), superposées à
   * droite de la ligne : masquées au repos, visibles au survol ou quand l'une d'elles a
   * le focus (elles restent dans l'ordre de tabulation).
   */
  renderActions?: (source: SidebarSource) => ReactNode;
  /** Affiche le bouton « Ajouter une source ». */
  onAdd?: () => void;
}

const KIND_ICONS: Record<SourceKind, IconName> = {
  library: "library",
  catalog: "catalog",
  folder: "folder",
};

const STATUS_ICONS: Record<Exclude<SourceStatus, "ok">, IconName> = {
  offline: "offline",
  error: "alert",
};

const NAV_KEYS = ["ArrowDown", "ArrowUp", "Home", "End"];

/** Pose l'infobulle du libellé complet seulement s'il est tronqué. */
function syncTitle(event: SyntheticEvent<HTMLButtonElement>, label: string) {
  const button = event.currentTarget;
  const text = button.querySelector<HTMLElement>(".fl-source__label");
  if (text && text.scrollWidth > text.clientWidth) {
    button.title = label;
  } else {
    button.removeAttribute("title");
  }
}

/**
 * Liste des sources dans l'ordre fourni par l'appelant. Chaque source est un bouton :
 * Tab passe de l'une à l'autre (et à leurs actions) ; les flèches haut/bas, Début et
 * Fin déplacent directement le focus entre les sources.
 */
export function SidebarSourceList({
  sources,
  selectedId,
  onSelect,
  title = t.sources.title,
  renderActions,
  onAdd,
}: SidebarSourceListProps) {
  const listRef = useRef<HTMLUListElement>(null);
  const titleId = useId();

  function handleKeyDown(event: KeyboardEvent<HTMLUListElement>) {
    if (!NAV_KEYS.includes(event.key) || !listRef.current) return;
    const items = Array.from(
      listRef.current.querySelectorAll<HTMLButtonElement>(".fl-source__button"),
    );
    const current = items.indexOf(document.activeElement as HTMLButtonElement);
    if (current === -1) return;
    event.preventDefault();
    const last = items.length - 1;
    const next =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? last
          : event.key === "ArrowDown"
            ? Math.min(last, current + 1)
            : Math.max(0, current - 1);
    items[next]?.focus();
  }

  return (
    <section className="fl-sources" aria-labelledby={titleId}>
      <h2 id={titleId} className="fl-sources__heading">
        {title}
      </h2>
      <ul className="fl-sources__list" ref={listRef} onKeyDown={handleKeyDown}>
        {sources.map((source) => {
          const selected = source.id === selectedId;
          const status = source.status ?? "ok";
          const actions = renderActions?.(source);
          return (
            <li
              key={source.id}
              className={cx("fl-source", selected && "fl-source--selected")}
            >
              <button
                type="button"
                className="fl-source__button"
                aria-current={selected ? "page" : undefined}
                onClick={() => onSelect(source.id)}
                onMouseEnter={(event) => syncTitle(event, source.label)}
                onFocus={(event) => syncTitle(event, source.label)}
              >
                <Icon name={KIND_ICONS[source.kind]} />
                <span className="fl-source__label">{source.label}</span>
                <span className="fl-visually-hidden">
                  , {t.sources.kind[source.kind]}
                </span>
                {status !== "ok" ? (
                  <span
                    className={`fl-source__meta fl-source__meta--${status}`}
                    title={t.sources.status[status]}
                  >
                    <Icon name={STATUS_ICONS[status]} size={16} />
                    <span className="fl-visually-hidden">
                      , {t.sources.status[status]}
                    </span>
                  </span>
                ) : (
                  source.count !== undefined && (
                    <span className="fl-source__meta">
                      <span aria-hidden="true">{source.count}</span>
                      <span className="fl-visually-hidden">
                        , {t.sources.count(source.count)}
                      </span>
                    </span>
                  )
                )}
              </button>
              {actions && <div className="fl-source__actions">{actions}</div>}
            </li>
          );
        })}
      </ul>
      {onAdd && (
        <Button variant="ghost" icon="plus" className="fl-sources__add" onClick={onAdd}>
          {t.sources.add}
        </Button>
      )}
    </section>
  );
}
