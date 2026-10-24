import {
  AlertTriangle,
  BarChart3,
  Clock,
  Coffee,
  DatabaseBackup,
  FolderOpen,
  Gamepad2,
  Hash,
  LayoutDashboard,
  Map as MapIcon,
  Puzzle,
  Power,
  RotateCw,
  SlidersHorizontal,
  Users,
} from "lucide-react";
import type { ServerTab } from "../../components/Sidebar";
import { StatusPill } from "../../components/StatusPill";
import { useT } from "../../i18n";
import { api } from "../../lib/api";
import { formatClock, loaderLabel } from "../../lib/format";
import type { ServerSummary } from "../../lib/types";
import type { AppData } from "../../lib/useAppData";
import { BackupsTab } from "./BackupsTab";
import { ModsTab } from "./ModsTab";
import { Overview } from "./Overview";
import { PlayersTab } from "./PlayersTab";
import { ServerSettingsTab } from "./ServerSettingsTab";
import { StatsTab } from "./StatsTab";

interface Props {
  server: ServerSummary;
  data: AppData;
  tab: ServerTab;
  /** Encendiendo, apagando o reiniciando desde esta ventana. */
  pending: boolean;
  onTab: (tab: ServerTab) => void;
  onStart: () => void;
  onStop: () => void;
  onRestart: () => void;
  onCommand: (command: string) => void;
  onOpenAppSettings: () => void;
  onDeleted: () => void;
  onError: (e: unknown) => void;
}

const TAB_ICONS: Record<ServerTab, typeof Users> = {
  overview: LayoutDashboard,
  players: Users,
  backups: DatabaseBackup,
  mods: Puzzle,
  stats: BarChart3,
  settings: SlidersHorizontal,
};

export function ServerView(props: Props) {
  const t = useT();
  const { server, data, tab, pending, onError } = props;
  const { config, status, live } = server;
  const id = config.id;
  const alive = status === "running" || status === "starting";
  const transitioning = status === "starting" || status === "stopping";
  const busy = server.busy || pending;
  const modded = config.loader.kind !== "vanilla";

  const tabs: ServerTab[] = [
    "overview",
    "players",
    "backups",
    ...(modded ? ["mods" as const] : []),
    "stats",
    "settings",
  ];
  const current = tabs.includes(tab) ? tab : "overview";
  const modBadge = (live.packUpdate ? 1 : 0) + live.modUpdates;

  let content;
  switch (current) {
    case "players":
      content = <PlayersTab key={id} server={server} onError={onError} />;
      break;
    case "backups":
      content = <BackupsTab key={id} server={server} onError={onError} onSaved={data.refresh} />;
      break;
    case "mods":
      content = <ModsTab key={id} server={server} onError={onError} onChanged={data.refresh} />;
      break;
    case "stats":
      content = <StatsTab key={id} server={server} />;
      break;
    case "settings":
      content = (
        <ServerSettingsTab
          key={id}
          server={server}
          hardware={data.hardware}
          network={data.network}
          platform={data.appInfo?.platform ?? "linux"}
          onError={onError}
          onSaved={data.refresh}
          onOpenAppSettings={props.onOpenAppSettings}
          onDeleted={props.onDeleted}
        />
      );
      break;
    default:
      content = (
        <Overview
          key={id}
          server={server}
          data={data}
          onTab={props.onTab}
          onStart={props.onStart}
          onCommand={props.onCommand}
          onError={onError}
        />
      );
  }

  return (
    <div className="page server-page">
      <header className="page-header">
        <div className="page-heading">
          <h1>{config.name}</h1>
          <div className="chips">
            <span className="chip">
              <Gamepad2 size={13} />
              {t.common.minecraft(config.mcVersion)}
            </span>
            {modded && <span className="chip">{loaderLabel(config.loader)}</span>}
            <span className="chip">
              <Coffee size={13} />
              {t.common.java(config.javaMajor)}
            </span>
            <span className="chip">
              <Hash size={13} />
              {t.common.port(config.port)}
            </span>
            {live.restartAt && alive && (
              <span className="chip">
                <Clock size={13} />
                {t.server.restartAt(formatClock(live.restartAt))}
              </span>
            )}
            {server.busy && (
              <span className="chip" data-tone="warning">
                <span className="spinner" />
                {t.server.busy}
              </span>
            )}
          </div>
        </div>
        <div className="page-actions">
          <StatusPill status={status} />
          {config.map.enabled && status === "running" && (
            <button
              className="btn btn-ghost btn-icon"
              onClick={() => api.openMap(id).catch(onError)}
              title={t.server.openMap}
              aria-label={t.server.openMap}
            >
              <MapIcon size={17} />
            </button>
          )}
          <button
            className="btn btn-ghost btn-icon"
            onClick={() => api.openServerFolder(id).catch(onError)}
            title={t.server.openFolder}
            aria-label={t.server.openFolder}
          >
            <FolderOpen size={17} />
          </button>
          <button
            className="btn btn-secondary"
            onClick={props.onRestart}
            disabled={status !== "running" || busy}
            title={t.server.restartTitle}
          >
            <RotateCw size={15} />
            {t.server.restart}
          </button>
          {alive || status === "stopping" ? (
            <button className="btn btn-danger" onClick={props.onStop} disabled={transitioning || busy}>
              <Power size={15} />
              {t.server.stop}
            </button>
          ) : (
            <button className="btn btn-primary" onClick={props.onStart} disabled={busy}>
              <Power size={15} />
              {t.server.start}
            </button>
          )}
        </div>
      </header>

      <nav className="tabs" role="tablist" aria-label={config.name}>
        {tabs.map((key) => {
          const Icon = TAB_ICONS[key];
          return (
            <button
              key={key}
              role="tab"
              aria-selected={current === key}
              data-selected={current === key || undefined}
              onClick={() => props.onTab(key)}
            >
              <Icon size={15} />
              {t.server.tabs[key]}
              {key === "mods" && modBadge > 0 && <span className="tab-badge">{modBadge}</span>}
              {key === "backups" && live.lastBackupError && (
                <AlertTriangle size={14} className="tab-warning" aria-label={t.backups.lastFailed} />
              )}
            </button>
          );
        })}
      </nav>

      {content}
    </div>
  );
}
