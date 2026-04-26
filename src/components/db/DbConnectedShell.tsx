import { Activity, Columns, Database, FolderTree, HardDrive, Table } from "lucide-react";
import type { ReactNode } from "react";
import { useState } from "react";

import { useI18n } from "../../i18n/useI18n";
import DbHeaderPicker, { type DbHeaderInstance } from "./DbHeaderPicker";
import DbSchemaTree, {
  type DbSchemaActions,
  type DbSchemaDatabase,
  type DbSchemaNode,
} from "./DbSchemaTree";
import DbTableBar from "./DbTableBar";
import type { DbKind } from "../../lib/types";

export type DbConnectedTab = "data" | "structure" | "schema";

export type DbConnectedSchema = {
  databases: DbSchemaDatabase[];
  selectedTableId: string | null;
  onSelectTable: (databaseName: string, node: DbSchemaNode) => void;
  onSelectDatabase?: (name: string) => void;
  /** PG-only: clicking a non-active schema in the tree fires
   *  this — the panel re-runs `postgresBrowse` with the new
   *  schema. MySQL / SQLite leave it undefined. */
  onSelectSchema?: (databaseName: string, schema: string) => void;
  /** Optional right-click actions for the wide-area schema tree. */
  actions?: DbSchemaActions;
};

type Crumb = {
  database?: string;
  schema?: string;
  table?: string;
  /** Right-aligned summary stat (e.g. "48,212 rows · 12 MB"). */
  stat?: ReactNode;
};

type HeaderStat = {
  icon: "database" | "disk" | "activity";
  label: ReactNode;
};

type Props = {
  kind: DbKind;
  current: DbHeaderInstance;
  otherInstances?: DbHeaderInstance[];
  onSwitchInstance?: (id: string) => void;
  onAddConnection: () => void;
  onDisconnect: () => void;
  /** Header stats chips — e.g. db count, size, ping. */
  headerStats?: HeaderStat[];
  tab: DbConnectedTab;
  onTabChange: (next: DbConnectedTab) => void;
  crumb: Crumb;
  /** Schema metadata — drives both the left-rail tree (wide) and the
   *  horizontal chip-bar at the top of the main area (narrow). */
  schema: DbConnectedSchema;
  /** Main body for the Data tab — typically <DbSqlEditor/> above <DbResultGrid/>. */
  dataTab: ReactNode;
  structureTab?: ReactNode;
  schemaTab?: ReactNode;
};

/**
 * Composed "connected" layout: header (picker + stats + segmented
 * Data/Structure/Schema) on top of a split (schema tree sidebar +
 * main body). At wide widths the sidebar holds the schema tree; at
 * narrow widths the sidebar disappears and a horizontal chip-bar
 * with the same data renders above the breadcrumb in the main area.
 * The sidebar can also be manually collapsed via the leading icon
 * in the crumb row.
 */
export default function DbConnectedShell({
  kind,
  current,
  otherInstances,
  onSwitchInstance,
  onAddConnection,
  onDisconnect,
  headerStats = [],
  tab,
  onTabChange,
  crumb,
  schema,
  dataTab,
  structureTab,
  schemaTab,
}: Props) {
  const { t } = useI18n();
  const [sidebarOpen, setSidebarOpen] = useState(true);

  return (
    <div className="db2">
      <header className="db2-head">
        <DbHeaderPicker
          kind={kind}
          current={current}
          others={otherInstances}
          onSwitch={onSwitchInstance}
          onAdd={onAddConnection}
          onDisconnect={onDisconnect}
        />
        {headerStats.length > 0 && (
          <div className="db2-stats">
            {headerStats.map((s, i) => {
              const Icon =
                s.icon === "disk" ? HardDrive : s.icon === "activity" ? Activity : Database;
              return (
                <span key={i} className="db2-stat">
                  <Icon size={10} />
                  {s.label}
                </span>
              );
            })}
          </div>
        )}
        <span className="db2-spacer" />
        <div className="db2-view-seg" role="tablist">
          <button
            type="button"
            className={"db2-seg" + (tab === "data" ? " on" : "")}
            onClick={() => onTabChange("data")}
          >
            <Table size={10} />
            <span className="label">{t("Data")}</span>
          </button>
          <button
            type="button"
            className={"db2-seg" + (tab === "structure" ? " on" : "")}
            onClick={() => onTabChange("structure")}
          >
            <Columns size={10} />
            <span className="label">{t("Structure")}</span>
          </button>
          <button
            type="button"
            className={"db2-seg" + (tab === "schema" ? " on" : "")}
            onClick={() => onTabChange("schema")}
          >
            <FolderTree size={10} />
            <span className="label">{t("Schema")}</span>
          </button>
        </div>
      </header>

      <div className="db2-split">
        {sidebarOpen && (
          <div className="db2-sidebar">
            <DbSchemaTree
              databases={schema.databases}
              selectedTableId={schema.selectedTableId}
              onSelectTable={schema.onSelectTable}
              onSelectDatabase={schema.onSelectDatabase}
              onSelectSchema={schema.onSelectSchema}
              actions={schema.actions}
            />
          </div>
        )}
        <div className="db2-main">
          <div className="db2-tablebar-host">
            <DbTableBar
              databases={schema.databases}
              selectedTableId={schema.selectedTableId}
              onSelectTable={schema.onSelectTable}
              onSelectDatabase={schema.onSelectDatabase}
            />
          </div>
          <div className="db2-crumb">
            <button
              type="button"
              className="mini-button mini-button--ghost db2-crumb__sidebar-toggle"
              onClick={() => setSidebarOpen((v) => !v)}
              title={t("Toggle schema tree")}
            >
              <FolderTree size={11} />
            </button>
            <span className="db2-crumb-path">
              {crumb.database && <span>{crumb.database}</span>}
              {crumb.schema && (
                <>
                  <span className="sep">/</span>
                  <span>{crumb.schema}</span>
                </>
              )}
              {crumb.table && (
                <>
                  <span className="sep">/</span>
                  <span className="last">{crumb.table}</span>
                </>
              )}
              {!crumb.database && !crumb.table && (
                <span className="sep">{t("(no table selected)")}</span>
              )}
            </span>
            {crumb.stat && <span className="db2-crumb-stat">{crumb.stat}</span>}
          </div>

          {tab === "data" && dataTab}
          {tab === "structure" && structureTab}
          {tab === "schema" && schemaTab}
        </div>
      </div>
    </div>
  );
}
