import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";

export interface ChartPoint {
  /** Milisegundos desde 1970. */
  t: number;
  v: number;
}

interface Props {
  points: ChartPoint[];
  from: number;
  to: number;
  /** Máximo fijo del eje (100 % de CPU, 20 TPS); si falta, se calcula. */
  yMax?: number;
  color: string;
  format: (v: number) => string;
  /** Etiquetas del eje vertical (por defecto, `format`). */
  formatAxis?: (v: number) => string;
  formatTime: (t: number) => string;
  ticks: number[];
  formatTick: (t: number) => string;
  /** Más separación que esta entre dos puntos es un hueco (servidor apagado). */
  gapMs: number;
  label: string;
  reference?: { value: number; label: string };
  height?: number;
}

const PAD = { left: 44, right: 14, top: 14, bottom: 26 };

/** Máximo "redondo" para el eje: 1, 2, 2,5 o 5 por potencia de 10. */
function niceMax(value: number): number {
  if (value <= 0) return 1;
  const power = 10 ** Math.floor(Math.log10(value));
  for (const step of [1, 2, 2.5, 5, 10]) {
    if (value <= step * power) return step * power;
  }
  return 10 * power;
}

/**
 * Gráfica de una sola medida: línea de 2 px con relleno suave, cuadrícula
 * discreta, huecos cuando el servidor estuvo apagado y lectura al pasar el
 * ratón (o con las flechas del teclado).
 */
export function LineChart(props: Props) {
  const { points, from, to, color, format, formatTime, ticks, formatTick, gapMs, label, reference } = props;
  const height = props.height ?? 190;
  const formatAxis = props.formatAxis ?? format;
  const ref = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [hover, setHover] = useState<number | null>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const peak = points.reduce((m, p) => Math.max(m, p.v), 0);
  const yMax = props.yMax ?? niceMax(peak * 1.1);
  const innerW = Math.max(width - PAD.left - PAD.right, 1);
  const innerH = height - PAD.top - PAD.bottom;
  const x = (t: number) => PAD.left + ((t - from) / Math.max(to - from, 1)) * innerW;
  const y = (v: number) => PAD.top + (1 - Math.min(Math.max(v, 0), yMax) / yMax) * innerH;
  const baseline = PAD.top + innerH;

  // Tramos continuos: el servidor apagado deja un hueco, no una rampa.
  const segments = useMemo(() => {
    const out: ChartPoint[][] = [];
    for (const p of points) {
      const last = out.at(-1);
      if (last && p.t - last[last.length - 1].t <= gapMs) last.push(p);
      else out.push([p]);
    }
    return out;
  }, [points, gapMs]);

  const gridValues = [0, yMax / 2, yMax];

  const nearest = (px: number) => {
    let best = 0;
    points.forEach((p, i) => {
      if (Math.abs(x(p.t) - px) < Math.abs(x(points[best].t) - px)) best = i;
    });
    return best;
  };

  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!points.length || !ref.current) return;
    setHover(nearest(e.clientX - ref.current.getBoundingClientRect().left));
  };

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (!points.length || (e.key !== "ArrowLeft" && e.key !== "ArrowRight")) return;
    e.preventDefault();
    const step = e.key === "ArrowLeft" ? -1 : 1;
    setHover((h) => Math.min(Math.max((h ?? points.length - 1) + step, 0), points.length - 1));
  };

  const active = hover !== null ? points[hover] : null;

  return (
    <div
      ref={ref}
      className="chart"
      style={{ height }}
      onPointerMove={onMove}
      onPointerLeave={() => setHover(null)}
      onKeyDown={onKey}
      onBlur={() => setHover(null)}
      tabIndex={points.length ? 0 : -1}
      role="img"
      aria-label={label}
    >
      {width > 0 && (
        <svg width={width} height={height} aria-hidden="true">
          {gridValues.map((v) => (
            <g key={v}>
              <line x1={PAD.left} x2={width - PAD.right} y1={y(v)} y2={y(v)} className="chart-grid" />
              <text x={PAD.left - 8} y={y(v)} className="chart-axis" textAnchor="end" dominantBaseline="middle">
                {formatAxis(v)}
              </text>
            </g>
          ))}
          {ticks
            .filter((t) => t >= from && t <= to)
            .map((t) => (
              <text key={t} x={x(t)} y={height - 6} className="chart-axis" textAnchor="middle">
                {formatTick(t)}
              </text>
            ))}
          {reference && reference.value <= yMax && (
            <g>
              <line
                x1={PAD.left}
                x2={width - PAD.right}
                y1={y(reference.value)}
                y2={y(reference.value)}
                className="chart-ref"
              />
              <text x={width - PAD.right} y={y(reference.value) - 5} className="chart-axis" textAnchor="end">
                {reference.label}
              </text>
            </g>
          )}
          {segments.map((seg) => {
            if (seg.length === 1) {
              return <circle key={seg[0].t} cx={x(seg[0].t)} cy={y(seg[0].v)} r={2.5} fill={color} />;
            }
            const line = seg.map((p, i) => `${i ? "L" : "M"}${x(p.t).toFixed(1)},${y(p.v).toFixed(1)}`).join("");
            const area = `${line}L${x(seg[seg.length - 1].t).toFixed(1)},${baseline}L${x(seg[0].t).toFixed(1)},${baseline}Z`;
            return (
              <g key={seg[0].t}>
                <path d={area} fill={color} opacity={0.1} />
                <path
                  d={line}
                  fill="none"
                  stroke={color}
                  strokeWidth={2}
                  strokeLinejoin="round"
                  strokeLinecap="round"
                />
              </g>
            );
          })}
          {active && (
            <>
              <line x1={x(active.t)} x2={x(active.t)} y1={PAD.top} y2={baseline} className="chart-cross" />
              <circle cx={x(active.t)} cy={y(active.v)} r={4.5} fill={color} stroke="var(--surface)" strokeWidth={2} />
            </>
          )}
        </svg>
      )}
      {active && (
        <div
          className="chart-tip"
          style={{ left: Math.min(Math.max(x(active.t), PAD.left + 60), width - 70), top: y(active.v) }}
        >
          <strong>{format(active.v)}</strong>
          <span>{formatTime(active.t)}</span>
        </div>
      )}
    </div>
  );
}
