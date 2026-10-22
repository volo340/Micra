import { ArrowUpCircle, Box, Cpu, Feather, Hammer, Layers, MemoryStick, Plus, Settings } from "lucide-react";
import { Logo } from "./Logo";
import { useT, type Dict } from "../i18n";
import { formatGb, loaderName } from "../lib/format";
import type { HardwareInfo, Loader, ServerSummary } from "../lib/types";

export type ServerTab = "overview" | "players" | "backups" | "mods" | "stats" | "settings";

export type View =
  | { kind: "home" }
  | { kind: "server"; id: string; tab: ServerTab }
  | { kind: "create" }
  | { kind: "settings" };

const LOADER_ICON: Record<Loader["kind"], typeof Box> = {
  vanilla: Box,
  fabric: Feather,
  quilt: Layers,
  forge: Hammer,
  neoForge: Hammer,
};

/** Apagado muestra la versión; cualquier otro estado lo dice con palabras. */
function itemMeta(s: ServerSummary, t: Dict): string {
  if (s.status === "stopped") return `${s.config.mcVersion} · ${loaderName(s.config.loader.kind)}`;
  if (s.status !== "running") return t.status[s.status];
  const n = s.players.length;
  return n === 0 ? t.status.running : `${t.status.running} · ${t.common.players(n)}`;
}

interface Props {
  servers: ServerSummary[];
  view: View;
  hardware: HardwareInfo | null;
  /** Versión nueva de Micra, si la hay. */
  update: string | null;
  onNavigate: (view: View) => void;
}

export function Sidebar({ servers, view, hardware, update, onNavigate }: Props) {
  const t = useT();
  return (
    <aside className="sidebar">
      <button className="brand" onClick={() => onNavigate({ kind: "home" })} aria-label={t.sidebar.home}>
        <Logo size={30} />
        <span className="brand-name">Micra</span>
      </button>

      <button className="btn btn-primary sidebar-create" onClick={() => onNavigate({ kind: "create" })}>
        <Plus size={16} strokeWidth={2.5} />
        {t.sidebar.newServer}
      </button>

      <div className="sidebar-section">
        <div className="sidebar-label">
          {t.sidebar.servers}
          <span className="count">{servers.length}</span>
        </div>
        <nav className="server-list">
          {servers.map((s) => {
            const Icon = LOADER_ICON[s.config.loader.kind];
            const active = view.kind === "server" && view.id === s.config.id;
            return (
              <button
                key={s.config.id}
                className="server-item"
                data-active={active || undefined}
                data-status={s.status}
                onClick={() => onNavigate({ kind: "server", id: s.config.id, tab: active ? view.tab : "overview" })}
              >
                <span className="server-glyph">
                  <Icon size={16} />
                  {s.status !== "stopped" && <span className="status-dot" />}
                </span>
                <span className="server-item-text">
                  <span className="server-item-name">{s.config.name}</span>
                  <span className="server-item-meta">{itemMeta(s, t)}</span>
                </span>
              </button>
            );
          })}
          {servers.length === 0 && <p className="sidebar-empty">{t.sidebar.empty}</p>}
        </nav>
      </div>

      <div className="sidebar-footer">
        {update && (
          <button className="update-pill" onClick={() => onNavigate({ kind: "settings" })}>
            <ArrowUpCircle size={16} />
            {t.sidebar.update(update)}
          </button>
        )}
        <button
          className="nav-item"
          data-active={view.kind === "settings" || undefined}
          onClick={() => onNavigate({ kind: "settings" })}
        >
          <Settings size={16} />
          {t.sidebar.settings}
        </button>
        {hardware && (
          <div className="sidebar-hw">
            <span>
              <Cpu size={13} />
              {t.common.threads(hardware.logicalCpus)}
            </span>
            <span>
              <MemoryStick size={13} />
              {formatGb(hardware.totalMemoryMb)}
            </span>
          </div>
        )}
      </div>
    </aside>
  );
}
