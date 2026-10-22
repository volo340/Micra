import type { ReactNode } from "react";
import { AlertTriangle, CheckCircle2, Cpu, Gauge, MemoryStick, OctagonAlert, Users } from "lucide-react";
import { Avatar } from "./Avatar";
import { Sparkline } from "./Sparkline";
import { useT } from "../i18n";
import { formatCores, formatGb, formatPercent, formatTps } from "../lib/format";
import type { StatsPoint } from "../lib/useAppData";

export const STATS_WINDOW = 60;

function Tile(props: { icon: ReactNode; label: string; aside?: string; children: ReactNode }) {
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
      </header>
      {props.children}
    </section>
  );
}

export function CpuTile(props: { points: StatsPoint[]; live: boolean; cores: number[] | null; logicalCpus: number }) {
  const t = useT();
  const last = props.points.at(-1);
  return (
    <Tile
      icon={<Cpu size={15} />}
      label={t.tiles.cpu}
      aside={props.cores ? t.common.cores(formatCores(props.cores)) : t.common.threads(props.logicalCpus)}
    >
      <div className="tile-value">{props.live && last ? formatPercent(last.cpu) : "—"}</div>
      <Sparkline
        values={props.live ? props.points.map((p) => p.cpu) : []}
        max={100}
        capacity={STATS_WINDOW}
        color="var(--series-cpu)"
        format={formatPercent}
        label={t.tiles.cpuChart}
      />
    </Tile>
  );
}

export function MemoryTile(props: { points: StatsPoint[]; live: boolean; maxMb: number }) {
  const t = useT();
  const last = props.points.at(-1);
  const values = props.live ? props.points.map((p) => p.memMb) : [];
  const ratio = last && props.live ? Math.min(last.memMb / props.maxMb, 1) : 0;
  return (
    <Tile icon={<MemoryStick size={15} />} label={t.tiles.memory} aside={t.tiles.memoryLimit(formatGb(props.maxMb))}>
      <div className="tile-value">{props.live && last ? formatGb(last.memMb) : "—"}</div>
      <div
        className="meter"
        role="meter"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(ratio * 100)}
        aria-label={t.tiles.memoryMeter}
      >
        <span style={{ width: `${ratio * 100}%` }} />
      </div>
      <Sparkline
        values={values}
        max={Math.max(props.maxMb, ...values) * 1.05}
        capacity={STATS_WINDOW}
        color="var(--series-ram)"
        format={formatGb}
        label={t.tiles.memoryChart}
        height={40}
      />
    </Tile>
  );
}

type TpsState = "good" | "lag" | "heavy";

const TPS_ICON = { good: CheckCircle2, lag: AlertTriangle, heavy: OctagonAlert };

/** 20 TPS es el ritmo normal del juego; por debajo de 15 se nota mucho. */
export const tpsState = (tps: number): TpsState => (tps >= 18 ? "good" : tps >= 15 ? "lag" : "heavy");

export function TpsTile({ tps, live }: { tps: number | null; live: boolean }) {
  const t = useT();
  const state = live && tps !== null ? tpsState(tps) : null;
  const Icon = state ? TPS_ICON[state] : null;
  const ratio = live && tps !== null ? Math.min(tps / 20, 1) : 0;
  return (
    <Tile icon={<Gauge size={15} />} label={t.tiles.tps} aside={t.tiles.tpsIdeal}>
      <div className="tile-value">{live && tps !== null ? formatTps(tps) : "—"}</div>
      <div
        className="meter"
        data-level={state ?? "idle"}
        role="meter"
        aria-valuemin={0}
        aria-valuemax={20}
        aria-valuenow={live && tps !== null ? tps : 0}
        aria-label={t.tiles.tps}
      >
        <span style={{ width: `${ratio * 100}%` }} />
      </div>
      <div className="tile-foot">
        {state && Icon ? (
          <span className="tps-state" data-state={state}>
            <Icon size={14} />
            {t.tiles.tpsStates[state]}
          </span>
        ) : (
          <span className="subtle">{live ? t.tiles.tpsWaiting : t.tiles.serverOff}</span>
        )}
      </div>
    </Tile>
  );
}

export function PlayersTile(props: { players: string[]; live: boolean }) {
  const t = useT();
  const shown = props.players.slice(0, 5);
  const extra = props.players.length - shown.length;
  return (
    <Tile icon={<Users size={15} />} label={t.tiles.players}>
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
          <span className="subtle">{props.live ? t.tiles.nobodyYet : t.tiles.serverOff}</span>
        )}
      </div>
    </Tile>
  );
}
