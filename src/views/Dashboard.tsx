import { Coffee, FolderOpen, Gamepad2, Hash, Power, RotateCw } from "lucide-react";
import { Console } from "../components/Console";
import { PlayersPanel } from "../components/PlayersPanel";
import { StatusPill } from "../components/StatusPill";
import { ConnectTile, CpuTile, MemoryTile, PlayersTile } from "../components/Tiles";
import { loaderLabel } from "../lib/format";
import type { LogLine } from "../lib/logs";
import type { HardwareInfo, NetworkInfo, ServerSummary } from "../lib/types";
import type { StatsPoint } from "../lib/useAppData";

interface Props {
  server: ServerSummary;
  lines: LogLine[];
  points: StatsPoint[];
  hardware: HardwareInfo | null;
  network: NetworkInfo | null;
  busy: boolean;
  onStart: () => void;
  onStop: () => void;
  onRestart: () => void;
  onCommand: (command: string) => void;
  onClear: () => void;
  onOpenFolder: () => void;
}

export function Dashboard(props: Props) {
  const { server, lines, points, hardware, network, busy } = props;
  const { config, status, players } = server;
  const live = status === "running" || status === "starting";
  const transitioning = status === "starting" || status === "stopping";

  return (
    <div className="page dashboard">
      <header className="page-header">
        <div className="page-heading">
          <h1>{config.name}</h1>
          <div className="chips">
            <span className="chip">
              <Gamepad2 size={13} />
              Minecraft {config.mcVersion}
            </span>
            {config.loader.kind !== "vanilla" && <span className="chip">{loaderLabel(config.loader)}</span>}
            <span className="chip">
              <Coffee size={13} />
              Java {config.javaMajor}
            </span>
            <span className="chip">
              <Hash size={13} />
              Puerto {config.port}
            </span>
          </div>
        </div>
        <div className="page-actions">
          <StatusPill status={status} />
          <button className="btn btn-ghost btn-icon" onClick={props.onOpenFolder} title="Abrir la carpeta del servidor">
            <FolderOpen size={17} />
          </button>
          <button
            className="btn btn-secondary"
            onClick={props.onRestart}
            disabled={status !== "running" || busy}
            title="Reiniciar el servidor"
          >
            <RotateCw size={15} />
            Reiniciar
          </button>
          {live || status === "stopping" ? (
            <button className="btn btn-danger" onClick={props.onStop} disabled={transitioning || busy}>
              <Power size={15} />
              Apagar
            </button>
          ) : (
            <button className="btn btn-primary" onClick={props.onStart} disabled={busy}>
              <Power size={15} />
              Encender
            </button>
          )}
        </div>
      </header>

      <div className="tiles">
        <CpuTile points={points} live={live} cores={config.cpu.cores} logicalCpus={hardware?.logicalCpus ?? 0} />
        <MemoryTile points={points} live={live} maxMb={config.memory.maxMb} />
        <PlayersTile players={players} live={status === "running"} />
        <ConnectTile ip={network?.localIp ?? null} port={config.port} />
      </div>

      <div className="workspace">
        <Console lines={lines} live={live} onCommand={props.onCommand} onClear={props.onClear} />
        <PlayersPanel players={players} live={status === "running"} onCommand={props.onCommand} />
      </div>
    </div>
  );
}
