import { Icon, type IconName } from "../../components";
import { t } from "../../i18n";
import { ROUTE_HASHES, type Route } from "../../router";
import "./AppNav.css";

const ITEMS: { route: keyof typeof ROUTE_HASHES; icon: IconName }[] = [
  { route: "home", icon: "library" },
  { route: "sources", icon: "catalog" },
  { route: "device", icon: "device" },
  { route: "settings", icon: "sliders" },
];

/** Liens de navigation entre les écrans (routage par hash). */
export function AppNav({ current }: { current: Route }) {
  return (
    <ul className="fl-nav">
      {ITEMS.map(({ route, icon }) => (
        <li key={route}>
          <a
            className="fl-nav__link"
            href={ROUTE_HASHES[route]}
            aria-current={current === route ? "page" : undefined}
          >
            <Icon name={icon} />
            {t.nav[route]}
          </a>
        </li>
      ))}
    </ul>
  );
}
