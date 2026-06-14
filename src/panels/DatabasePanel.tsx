import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import type { DbProduct, TabState } from "../lib/types";
import { DATABASE_TOOL_KINDS } from "../lib/types";
import { DB_KIND_META } from "../lib/rightToolMeta";
import { useI18n } from "../i18n/useI18n";
import { useTabStore } from "../stores/useTabStore";
import { useDetectedServicesStore } from "../stores/useDetectedServicesStore";
import ConnectSplash from "../components/ConnectSplash";
import PanelSkeleton from "../components/PanelSkeleton";
import "../styles/db-panel.css";

// One unified entry for the relational database clients. The tool strip
// shows a single "Database" button; this container renders a product
// switcher and routes the active tab's `dbKind` to the matching client —
// the same shape as WebServerPanel switching nginx / apache / caddy.
// Redis keeps its own strip entry (key-value model ≠ relational grid).

const MySqlPanel = lazy(() => import("./MySqlPanel"));
const PostgresPanel = lazy(() => import("./PostgresPanel"));
const SqlitePanel = lazy(() => import("./SqlitePanel"));
const SqlServerPanel = lazy(() => import("./SqlServerPanel"));
const InfluxPanel = lazy(() => import("./InfluxPanel"));
const RemoteSqlPanel = lazy(() => import("./RemoteSqlPanel"));

type Props = {
  tab: TabState | null;
  onConnectSaved: (index: number) => void;
  onNewConnection: () => void;
};

export default function DatabasePanel({
  tab,
  onConnectSaved,
  onNewConnection,
}: Props) {
  const { t } = useI18n();

  // Source of truth is the persisted `tab.dbKind`; a local fallback covers
  // the (defensive) no-tab path so the switcher still works.
  const storeKind = tab?.dbKind;
  const [localKind, setLocalKind] = useState<DbProduct>(storeKind ?? "mysql");
  const activeKind: DbProduct = storeKind ?? localKind;

  const instances = useDetectedServicesStore((s) =>
    tab ? s.instancesByTab[tab.id]?.instances : undefined,
  );
  const runningKinds = useMemo(() => {
    const set = new Set<string>();
    for (const inst of instances ?? []) set.add(inst.kind);
    return set;
  }, [instances]);

  const pick = (k: DbProduct) => {
    setLocalKind(k);
    if (tab) useTabStore.getState().updateTab(tab.id, { dbKind: k });
  };

  return (
    <div className="db-stack">
      <DbProductSwitcher
        active={activeKind}
        running={runningKinds}
        onPick={pick}
        t={t}
      />
      <div className="db-stack__body">
        <Suspense fallback={<PanelSkeleton variant="grid" rows={8} />}>
          {renderProduct(activeKind, tab, onConnectSaved, onNewConnection, t)}
        </Suspense>
      </div>
    </div>
  );
}

function renderProduct(
  kind: DbProduct,
  tab: TabState | null,
  onConnectSaved: (index: number) => void,
  onNewConnection: () => void,
  t: (s: string) => string,
) {
  const splash = () => (
    <DbSplash kind={kind} onConnectSaved={onConnectSaved} onNewConnection={onNewConnection} t={t} />
  );
  switch (kind) {
    case "sqlite":
      // SQLite handles a null tab itself (scans the connected host / local).
      return <SqlitePanel key={tab?.id ?? "no-tab"} tab={tab} />;
    case "mysql":
      return tab ? <MySqlPanel key={tab.id} tab={tab} /> : splash();
    case "postgres":
      return tab ? <PostgresPanel key={tab.id} tab={tab} /> : splash();
    case "sqlserver":
      return <SqlServerPanel key={tab?.id ?? "no-tab"} tab={tab} />;
    case "influx":
      return <InfluxPanel key={tab?.id ?? "no-tab"} tab={tab} />;
    case "oracle":
    case "dameng":
      return <RemoteSqlPanel key={`${kind}-${tab?.id ?? "no-tab"}`} tab={tab} kind={kind} />;
  }
}

function DbSplash({
  kind,
  onConnectSaved,
  onNewConnection,
  t,
}: {
  kind: DbProduct;
  onConnectSaved: (index: number) => void;
  onNewConnection: () => void;
  t: (s: string) => string;
}) {
  const meta = DB_KIND_META[kind];
  const Icon = meta.icon;
  return (
    <ConnectSplash
      icon={<Icon size={22} strokeWidth={1.6} />}
      title={meta.label}
      subtitle={t(meta.splashSubtitle)}
      tintVar={meta.tintVar}
      tagLabel={t("SSH")}
      onConnectSaved={onConnectSaved}
      onNewConnection={onNewConnection}
    />
  );
}

function DbProductSwitcher({
  active,
  running,
  onPick,
  t,
}: {
  active: DbProduct;
  running: ReadonlySet<string>;
  onPick: (k: DbProduct) => void;
  t: (s: string) => string;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [overflow, setOverflow] = useState({ left: false, right: false });

  const updateOverflow = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const { scrollLeft, scrollWidth, clientWidth } = el;
    setOverflow({
      left: scrollLeft > 0,
      right: scrollLeft + clientWidth < scrollWidth - 1,
    });
  }, []);

  // Recompute the arrow state whenever the bar or its content is resized
  // (panel width changes, fonts load, items added later).
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    updateOverflow();
    const ro = new ResizeObserver(updateOverflow);
    ro.observe(el);
    for (const child of Array.from(el.children)) ro.observe(child);
    return () => ro.disconnect();
  }, [updateOverflow]);

  // Keep the active pill in view when switching products via other entry
  // points (palette, persisted tab) so the selection is never off-screen.
  useEffect(() => {
    const el = scrollRef.current;
    const activeEl = el?.querySelector('[aria-selected="true"]');
    activeEl?.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "nearest" });
  }, [active]);

  function step(direction: -1 | 1) {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollBy({ left: direction * Math.max(el.clientWidth * 0.7, 120), behavior: "smooth" });
  }

  function onWheel(event: React.WheelEvent<HTMLDivElement>) {
    const el = scrollRef.current;
    if (!el) return;
    // Translate a vertical wheel into horizontal scroll so users without a
    // horizontal-scroll device can still reach overflowing products.
    if (event.deltaY !== 0 && el.scrollWidth > el.clientWidth) {
      el.scrollLeft += event.deltaY;
    }
  }

  const scrollable = overflow.left || overflow.right;

  return (
    <div className="db-switcher">
      {scrollable && (
        <button
          type="button"
          className="db-switcher__arrow"
          disabled={!overflow.left}
          title={t("Scroll left")}
          aria-label={t("Scroll left")}
          onClick={() => step(-1)}
        >
          <ChevronLeft size={14} />
        </button>
      )}
      <div
        className="db-switcher__scroll"
        role="tablist"
        ref={scrollRef}
        onScroll={updateOverflow}
        onWheel={onWheel}
      >
        {DATABASE_TOOL_KINDS.map((kind) => {
          const meta = DB_KIND_META[kind];
          const Icon = meta.icon;
          const isActive = kind === active;
          const isRunning = running.has(kind);
          return (
            <button
              key={kind}
              type="button"
              role="tab"
              aria-selected={isActive}
              className={
                "db-switcher__btn" +
                (isActive ? " is-active" : "") +
                (meta.available ? "" : " db-switcher__btn--soon")
              }
              style={{ ["--db-tint" as string]: meta.tintVar }}
              onClick={() => onPick(kind)}
              title={meta.label}
            >
              <span className="db-switcher__icon">
                <Icon size={13} />
              </span>
              <span className="db-switcher__label">{meta.label}</span>
              {isRunning && (
                <span className="db-switcher__dot" title={t("Running")} />
              )}
              {!meta.available && (
                <span className="db-switcher__soon">{t("soon")}</span>
              )}
            </button>
          );
        })}
      </div>
      {scrollable && (
        <button
          type="button"
          className="db-switcher__arrow"
          disabled={!overflow.right}
          title={t("Scroll right")}
          aria-label={t("Scroll right")}
          onClick={() => step(1)}
        >
          <ChevronRight size={14} />
        </button>
      )}
    </div>
  );
}
