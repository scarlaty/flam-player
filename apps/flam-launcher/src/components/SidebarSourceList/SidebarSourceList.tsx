import { useRef, type KeyboardEvent } from "react";
import { t } from "../../i18n";
import { Button } from "../Button/Button";
import { cx } from "../cx";
import { Icon, type IconName } from "../Icon";
import "./SidebarSourceList.css";

export type SourceKind = "library" | "catalog" | "folder";

export interface SidebarSource {
  id: string;
  label: string;
  kind: SourceKind;
  /** Nombre d'histoires, si connu. */
  count?: number;
  /** Source injoignable (catalogue hors ligne, dossier absent…). */
  unavailable?: boolean;
}

export interface SidebarSourceListProps {
  sources: SidebarSource[];
  selectedId?: string;
  onSelect: (id: string) => void;
  /** Affiche le bouton « Ajouter une source ». */
  onAdd?: () => void;
}

const KIND_ICONS: Record<SourceKind, IconName> = {
  library: "library",
  catalog: "catalog",
  folder: "folder",
};

const KIND_ORDER: SourceKind[] = ["library", "catalog", "folder"];

/**
 * Liste des sources, regroupées par type. Tab entre dans la liste, les flèches
 * haut/bas (et Début/Fin) déplacent le focus d'une source à l'autre.
 */
export function SidebarSourceList({
  sources,
  selectedId,
  onSelect,
  onAdd,
}: SidebarSourceListProps) {
  const listRef = useRef<HTMLDivElement>(null);

  function handleKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const keys = ["ArrowDown", "ArrowUp", "Home", "End"];
    if (!keys.includes(event.key) || !listRef.current) return;
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

  const groups = KIND_ORDER.map((kind) => ({
    kind,
    items: sources.filter((source) => source.kind === kind),
  })).filter((group) => group.items.length > 0);

  return (
    <div className="fl-sources" ref={listRef} onKeyDown={handleKeyDown}>
      {groups.map((group) => (
        <section
          key={group.kind}
          className="fl-sources__group"
          aria-labelledby={`fl-sources-${group.kind}`}
        >
          <h2 id={`fl-sources-${group.kind}`} className="fl-sources__heading">
            {t.sources.kind[group.kind]}
          </h2>
          <ul className="fl-sources__list">
            {group.items.map((source) => {
              const selected = source.id === selectedId;
              return (
                <li key={source.id}>
                  <button
                    type="button"
                    className={cx(
                      "fl-source__button",
                      selected && "fl-source__button--selected",
                    )}
                    aria-current={selected ? "page" : undefined}
                    onClick={() => onSelect(source.id)}
                  >
                    <Icon name={KIND_ICONS[source.kind]} />
                    <span className="fl-source__label">{source.label}</span>
                    {source.unavailable ? (
                      <span className="fl-source__meta fl-source__meta--warning">
                        <Icon name="alert" size={16} />
                        <span className="fl-visually-hidden">
                          {t.sources.unavailable}
                        </span>
                      </span>
                    ) : (
                      source.count !== undefined && (
                        <span className="fl-source__meta">
                          <span aria-hidden="true">{source.count}</span>
                          <span className="fl-visually-hidden">
                            {t.sources.count(source.count)}
                          </span>
                        </span>
                      )
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
        </section>
      ))}
      {onAdd && (
        <Button variant="ghost" icon="plus" className="fl-sources__add" onClick={onAdd}>
          {t.sources.add}
        </Button>
      )}
    </div>
  );
}
