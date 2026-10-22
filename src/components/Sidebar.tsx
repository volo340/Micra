import { Box, Cpu, Feather, Hammer, Layers, MemoryStick, Plus, Settings } from "lucide-react";
import { Logo } from "./Logo";
import { STATUS_LABEL } from "./StatusPill";
import { formatGb, loaderName } from "../lib/format";
import type { HardwareInfo, Loader, ServerSummary } from "../lib/types";

export type View = { kind: "home" } | { kind: "server"; id: string } | { kind: "create" } | { kind: "settings" };

const LOADER_ICON: Record<Loader["kind"], typeof Box> = {
  vanilla: Box,
  fabric: Feather,
  quilt: Layers,
  forge: Hammer,
  neoForge: Hammer,
};

/** Apagado muestra la versión; cualquier otro estado lo dice con palabras. */
function itemMeta(s: ServerSummary): string {
  if (s.status === "stopped") return `${s.config.mcVersion} · ${loaderName(s.config.loader.kind)}`;
  if (s.status !== "running") return STATUS_LABEL[s.status];
  const n = s.players.length;
  return n === 0 ? "En línea" : `En línea · ${n} ${n === 1 ? "jugador" : "jugadores"}`;
}

interface Props {
  servers: ServerSummary[];
  view: View;
  hardware: HardwareInfo | null;
  onNavigate: (view: View) => void;
}

export function Sidebar({ servers, view, hardware, onNavigate }: Props) {
  return (
    <aside className="sidebar">
      <button className="brand" onClick={() => onNavigate({ kind: "home" })} aria-label="Inicio">
        <Logo size={30} />
        <span className="brand-name">Micra</span>
      </button>

      <button className="btn btn-primary sidebar-create" onClick={() => onNavigate({ kind: "create" })}>
        <Plus size={16} strokeWidth={2.5} />
        Nuevo servidor
      </button>

      <div className="sidebar-section">
        <div className="sidebar-label">
          Servidores
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
                onClick={() => onNavigate({ kind: "server", id: s.config.id })}
              >
                <span className="server-glyph">
                  <Icon size={16} />
                  {s.status !== "stopped" && <span className="status-dot" />}
                </span>
                <span className="server-item-text">
                  <span className="server-item-name">{s.config.name}</span>
                  <span className="server-item-meta">{itemMeta(s)}</span>
                </span>
              </button>
            );
          })}
          {servers.length === 0 && <p className="sidebar-empty">Aún no tienes servidores.</p>}
        </nav>
      </div>

      <div className="sidebar-footer">
        <button
          className="nav-item"
          data-active={view.kind === "settings" || undefined}
          onClick={() => onNavigate({ kind: "settings" })}
        >
          <Settings size={16} />
          Ajustes
        </button>
        {hardware && (
          <div className="sidebar-hw">
            <span>
              <Cpu size={13} />
              {hardware.logicalCpus} hilos
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
