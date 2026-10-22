import { useState, type ReactNode } from "react";
import { Check, Copy, Cpu, Globe2, MemoryStick, Users } from "lucide-react";
import { Avatar } from "./Avatar";
import { Sparkline } from "./Sparkline";
import { formatCores, formatGb, formatPercent } from "../lib/format";
import type { StatsPoint } from "../lib/useAppData";

export const STATS_WINDOW = 60;

function Tile(props: { icon: ReactNode; label: string; aside?: string; action?: ReactNode; children: ReactNode }) {
  return (
    <section className="card tile">
      <header className="tile-head">
        <span className="tile-label">
          {props.icon}
          {props.label}
        </span>
        {props.aside && (
          <span className="tile-aside" title={props.aside}>
            {props.aside}
          </span>
        )}
        {props.action}
      </header>
      {props.children}
    </section>
  );
}

export function CpuTile(props: { points: StatsPoint[]; live: boolean; cores: number[] | null; logicalCpus: number }) {
  const last = props.points.at(-1);
  return (
    <Tile
      icon={<Cpu size={15} />}
      label="CPU"
      aside={props.cores ? `Núcleos ${formatCores(props.cores)}` : `${props.logicalCpus} hilos`}
    >
      <div className="tile-value">{props.live && last ? formatPercent(last.cpu) : "—"}</div>
      <Sparkline
        values={props.live ? props.points.map((p) => p.cpu) : []}
        max={100}
        capacity={STATS_WINDOW}
        color="var(--series-cpu)"
        format={formatPercent}
        label="Uso de CPU"
      />
    </Tile>
  );
}

export function MemoryTile(props: { points: StatsPoint[]; live: boolean; maxMb: number }) {
  const last = props.points.at(-1);
  const values = props.live ? props.points.map((p) => p.memMb) : [];
  const ratio = last && props.live ? Math.min(last.memMb / props.maxMb, 1) : 0;
  return (
    <Tile icon={<MemoryStick size={15} />} label="Memoria" aside={`Límite ${formatGb(props.maxMb)}`}>
      <div className="tile-value">{props.live && last ? formatGb(last.memMb) : "—"}</div>
      <div
        className="meter"
        role="meter"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(ratio * 100)}
        aria-label="Memoria usada sobre la asignada"
      >
        <span style={{ width: `${ratio * 100}%` }} />
      </div>
      <Sparkline
        values={values}
        max={Math.max(props.maxMb, ...values) * 1.05}
        capacity={STATS_WINDOW}
        color="var(--series-ram)"
        format={formatGb}
        label="Memoria usada"
        height={40}
      />
    </Tile>
  );
}

export function PlayersTile(props: { players: string[]; live: boolean }) {
  const shown = props.players.slice(0, 5);
  const extra = props.players.length - shown.length;
  return (
    <Tile icon={<Users size={15} />} label="Jugadores">
      <div className="tile-value">{props.live ? props.players.length : "—"}</div>
      <div className="tile-foot">
        {props.players.length > 0 ? (
          <div className="avatar-stack">
            {shown.map((p) => (
              <Avatar key={p} name={p} />
            ))}
            {extra > 0 && <span className="count">+{extra}</span>}
          </div>
        ) : (
          <span className="subtle">{props.live ? "Nadie conectado todavía" : "El servidor está apagado"}</span>
        )}
      </div>
    </Tile>
  );
}

export function ConnectTile(props: { ip: string | null; port: number }) {
  const [copied, setCopied] = useState(false);
  const address = `${props.ip ?? "localhost"}:${props.port}`;

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(address);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      /* el portapapeles puede no estar disponible; la dirección sigue visible */
    }
  };

  const label = copied ? "Dirección copiada" : "Copiar dirección";
  return (
    <Tile
      icon={<Globe2 size={15} />}
      label="Dirección para conectarse"
      action={
        <button
          className="btn btn-ghost btn-icon btn-sm copy-btn"
          data-done={copied || undefined}
          onClick={copy}
          title={label}
          aria-label={label}
        >
          {copied ? <Check size={15} /> : <Copy size={15} />}
        </button>
      }
    >
      <div className="connect-address mono selectable" title={address}>
        {address}
      </div>
      <p className="tile-note">
        Funciona para quien esté en tu misma red (Wi-Fi o cable). La conexión desde internet llegará con la apertura
        automática de puertos.
      </p>
    </Tile>
  );
}
