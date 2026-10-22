import { useEffect, useState, type ReactNode } from "react";
import { BarChart3, Cpu, Gauge, MemoryStick, Users } from "lucide-react";
import { LineChart, type ChartPoint } from "../../components/LineChart";
import { Segmented } from "../../components/ui";
import { useT } from "../../i18n";
import { api } from "../../lib/api";
import {
  formatClock,
  formatDateTime,
  formatGb,
  formatNumber,
  formatPercent,
  formatShortDate,
  formatTps,
} from "../../lib/format";
import type { ServerSummary, StatPoint, StatsRange } from "../../lib/types";

const RANGES: StatsRange[] = ["hour", "day", "week", "month"];

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const SPAN: Record<StatsRange, number> = { hour: HOUR, day: DAY, week: 7 * DAY, month: 30 * DAY };
/** El backend da como mucho 240 puntos: más separación que esto es un hueco. */
const GAP: Record<StatsRange, number> = { hour: 3 * MINUTE, day: 15 * MINUTE, week: 2 * HOUR, month: 8 * HOUR };

/** Marcas del eje horizontal alineadas a horas o días locales. */
function makeTicks(from: number, to: number, range: StatsRange): number[] {
  const out: number[] = [];
  if (range === "hour" || range === "day") {
    const step = range === "hour" ? 15 * MINUTE : 4 * HOUR;
    const d = new Date(from);
    d.setSeconds(0, 0);
    if (range === "hour") d.setMinutes(Math.ceil(d.getMinutes() / 15) * 15);
    else d.setHours(Math.ceil((d.getHours() + d.getMinutes() / 60) / 4) * 4, 0);
    for (let t = d.getTime(); t <= to; t += step) out.push(t);
    return out;
  }
  const days = range === "week" ? 1 : 7;
  const d = new Date(from);
  d.setHours(24, 0, 0, 0);
  for (; d.getTime() <= to; d.setDate(d.getDate() + days)) out.push(d.getTime());
  return out;
}

/** Máximo del eje de RAM en GB redondos. */
function memoryAxis(limitMb: number, peakMb: number): number {
  const gb = (Math.max(limitMb, peakMb) * 1.05) / 1024;
  const steps = [1, 2, 4, 5, 6, 8, 10, 12, 16, 20, 24, 32, 48, 64, 96, 128];
  return (steps.find((s) => s >= gb) ?? Math.ceil(gb)) * 1024;
}

const average = (values: number[]) => values.reduce((a, b) => a + b, 0) / Math.max(values.length, 1);

function ChartCard(props: { icon: ReactNode; title: string; summary: string | null; children: ReactNode }) {
  return (
    <section className="card chart-card">
      <header className="chart-head">
        <span className="card-title">
          {props.icon}
          {props.title}
        </span>
        {props.summary && <span className="chart-summary">{props.summary}</span>}
      </header>
      {props.children}
    </section>
  );
}

/** Pestaña Estadísticas: CPU, RAM, jugadores y TPS guardados, una gráfica por medida. */
export function StatsTab({ server }: { server: ServerSummary }) {
  const t = useT();
  const id = server.config.id;
  const running = server.status === "running";
  const [range, setRange] = useState<StatsRange>("day");
  const [points, setPoints] = useState<StatPoint[] | null>(null);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    let cancelled = false;
    const load = () =>
      api
        .serverStats(id, range)
        .then((p) => {
          if (cancelled) return;
          setPoints(p);
          setNow(Date.now());
        })
        .catch(() => !cancelled && setPoints([]));
    setPoints(null);
    load();
    // Encendido, se guarda un punto por minuto.
    const timer = running ? window.setInterval(load, MINUTE) : undefined;
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [id, range, running]);

  const from = now - SPAN[range];
  const ticks = makeTicks(from, now, range);
  const formatTick = range === "hour" || range === "day" ? formatClock : formatShortDate;
  const list = points ?? [];
  const series = (pick: (p: StatPoint) => number | null): ChartPoint[] =>
    list.flatMap((p) => {
      const v = pick(p);
      return v === null ? [] : [{ t: p.ts * 1000, v }];
    });

  const cpu = series((p) => p.cpu);
  const ram = series((p) => p.ramMb);
  const players = series((p) => p.players);
  const tps = series((p) => p.tps);
  const peakPlayers = players.reduce((m, p) => Math.max(m, p.v), 0);
  const limit = server.config.memory.maxMb;
  const common = { from, to: now, ticks, formatTick, formatTime: formatDateTime, gapMs: GAP[range] };
  const values = (s: ChartPoint[]) => s.map((p) => p.v);

  return (
    <div className="tab-body">
      <div className="stats-toolbar">
        <Segmented
          label={t.stats.range}
          value={range}
          onChange={setRange}
          options={RANGES.map((r) => ({ value: r, label: t.stats.ranges[r] }))}
        />
        <span className="subtle">{t.stats.perMinute}</span>
      </div>

      {points !== null && list.length === 0 ? (
        <section className="card">
          <div className="empty">
            <span className="empty-icon">
              <BarChart3 size={20} />
            </span>
            <strong>{t.stats.empty}</strong>
            <span>{t.stats.emptyHint}</span>
          </div>
        </section>
      ) : (
        <div className="charts">
          <ChartCard
            icon={<Cpu size={16} />}
            title={t.stats.cpu}
            summary={
              cpu.length
                ? t.stats.avgMax(formatPercent(average(values(cpu))), formatPercent(Math.max(...values(cpu))))
                : null
            }
          >
            <LineChart
              {...common}
              points={cpu}
              yMax={100}
              color="var(--series-cpu)"
              format={formatPercent}
              label={t.stats.chartLabel(t.stats.cpu, t.stats.ranges[range])}
            />
          </ChartCard>

          <ChartCard
            icon={<MemoryStick size={16} />}
            title={t.stats.memory}
            summary={
              ram.length ? t.stats.avgMax(formatGb(average(values(ram))), formatGb(Math.max(...values(ram)))) : null
            }
          >
            <LineChart
              {...common}
              points={ram}
              yMax={memoryAxis(limit, Math.max(0, ...values(ram)))}
              color="var(--series-ram)"
              format={formatGb}
              reference={{ value: limit, label: t.stats.limit(formatGb(limit)) }}
              label={t.stats.chartLabel(t.stats.memory, t.stats.ranges[range])}
            />
          </ChartCard>

          <ChartCard
            icon={<Users size={16} />}
            title={t.stats.players}
            summary={players.length ? t.stats.peakPlayers(peakPlayers) : null}
          >
            <LineChart
              {...common}
              points={players}
              yMax={Math.max(2, Math.ceil((peakPlayers + 1) / 2) * 2)}
              color="var(--series-players)"
              format={(v) => formatNumber(Math.round(v))}
              label={t.stats.chartLabel(t.stats.players, t.stats.ranges[range])}
            />
          </ChartCard>

          <ChartCard
            icon={<Gauge size={16} />}
            title={t.stats.tps}
            summary={
              tps.length ? t.stats.avgMin(formatTps(average(values(tps))), formatTps(Math.min(...values(tps)))) : null
            }
          >
            <LineChart
              {...common}
              points={tps}
              yMax={20}
              color="var(--series-tps)"
              format={formatTps}
              formatAxis={formatNumber}
              label={t.stats.chartLabel(t.stats.tps, t.stats.ranges[range])}
            />
          </ChartCard>
        </div>
      )}
    </div>
  );
}
