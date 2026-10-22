import { useEffect, useRef, useState, type PointerEvent } from "react";

interface Props {
  values: number[];
  max: number;
  color: string;
  /** Puntos que caben en la ventana (1 por segundo). */
  capacity: number;
  format: (v: number) => string;
  label: string;
  height?: number;
}

const PAD = 6;

function ago(seconds: number) {
  return seconds === 0 ? "ahora" : `hace ${seconds} s`;
}

/**
 * Línea de tendencia de una sola serie: línea de 2 px, relleno al 10 %,
 * punto final con anillo del color de la superficie y lectura al pasar el ratón.
 */
export function Sparkline({ values, max, color, capacity, format, label, height = 52 }: Props) {
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

  const n = values.length;
  const step = (width - PAD * 2) / Math.max(capacity - 1, 1);
  const baseline = height - PAD;
  const xs = values.map((_, i) => width - PAD - (n - 1 - i) * step);
  const ys = values.map((v) => PAD + (1 - Math.min(Math.max(v, 0), max) / max) * (height - PAD * 2));
  const line = xs.map((x, i) => `${i ? "L" : "M"}${x.toFixed(1)},${ys[i].toFixed(1)}`).join("");
  const area = n > 1 ? `${line}L${xs[n - 1].toFixed(1)},${baseline}L${xs[0].toFixed(1)},${baseline}Z` : "";

  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!n || !ref.current) return;
    const x = e.clientX - ref.current.getBoundingClientRect().left;
    let best = 0;
    xs.forEach((px, i) => {
      if (Math.abs(px - x) < Math.abs(xs[best] - x)) best = i;
    });
    setHover(best);
  };

  const dot = hover ?? (n ? n - 1 : null);
  const current = n ? format(values[n - 1]) : "sin datos";

  return (
    <div
      ref={ref}
      className="spark"
      style={{ height }}
      onPointerMove={onMove}
      onPointerLeave={() => setHover(null)}
      onFocus={() => n && setHover(n - 1)}
      onBlur={() => setHover(null)}
      tabIndex={n ? 0 : -1}
      role="img"
      aria-label={`${label}, último minuto. Valor actual: ${current}`}
    >
      {width > 0 && (
        <svg width={width} height={height} aria-hidden="true">
          <line x1={0} x2={width} y1={baseline} y2={baseline} className="spark-base" />
          {n > 1 && <path d={area} fill={color} opacity={0.1} />}
          {n > 1 && (
            <path d={line} fill="none" stroke={color} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" />
          )}
          {hover !== null && <line x1={xs[hover]} x2={xs[hover]} y1={0} y2={baseline} className="spark-cross" />}
          {dot !== null && (
            <circle cx={xs[dot]} cy={ys[dot]} r={4} fill={color} stroke="var(--surface)" strokeWidth={2} />
          )}
        </svg>
      )}
      {hover !== null && (
        <div className="spark-tip" style={{ left: Math.min(Math.max(xs[hover], 48), width - 48) }}>
          <strong>{format(values[hover])}</strong>
          <span>{ago(n - 1 - hover)}</span>
        </div>
      )}
    </div>
  );
}
