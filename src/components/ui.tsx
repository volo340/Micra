// Piezas pequeñas que comparten varias pantallas.
import { useState, type ReactNode } from "react";
import { AlertTriangle, Check, CheckCircle2, Copy, Info, ShieldAlert } from "lucide-react";
import { useT } from "../i18n";
import { formatPercent } from "../lib/format";

type Tone = "info" | "warning" | "critical" | "success";

const BANNER_ICONS = { info: Info, warning: AlertTriangle, critical: ShieldAlert, success: CheckCircle2 };

/** Aviso con icono (el color nunca va solo) y una acción opcional. */
export function Banner({ tone = "info", children, action }: { tone?: Tone; children: ReactNode; action?: ReactNode }) {
  const Icon = BANNER_ICONS[tone];
  return (
    <div className="banner" data-tone={tone} role={tone === "critical" ? "alert" : undefined}>
      <Icon size={17} className="banner-icon" />
      <div className="banner-text">{children}</div>
      {action && <div className="banner-action">{action}</div>}
    </div>
  );
}

export function CopyButton({ text, label }: { text: string; label?: string }) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      /* el portapapeles puede no estar disponible; el texto sigue visible */
    }
  };
  const title = copied ? t.common.copied : (label ?? t.common.copy);
  return (
    <button
      type="button"
      className="btn btn-ghost btn-icon btn-sm copy-icon"
      data-done={copied || undefined}
      onClick={copy}
      title={title}
      aria-label={title}
    >
      {copied ? <Check size={15} /> : <Copy size={15} />}
    </button>
  );
}

/** Interruptor con su texto y una explicación debajo. */
export function SwitchRow(props: {
  label: ReactNode;
  hint?: ReactNode;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="switch-row" data-disabled={props.disabled || undefined}>
      <span className="switch-row-text">
        <span className="switch-row-label">{props.label}</span>
        {props.hint && <span className="field-hint">{props.hint}</span>}
      </span>
      <input
        type="checkbox"
        className="switch"
        checked={props.checked}
        disabled={props.disabled}
        onChange={(e) => props.onChange(e.target.checked)}
      />
    </label>
  );
}

/** Grupo de opciones excluyentes con aspecto de pestañas pequeñas. */
export function Segmented<T extends string>(props: {
  value: T;
  options: { value: T; label: string; disabled?: boolean }[];
  onChange: (value: T) => void;
  label: string;
}) {
  return (
    <div className="segmented" role="radiogroup" aria-label={props.label}>
      {props.options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={props.value === o.value}
          data-selected={props.value === o.value || undefined}
          disabled={o.disabled}
          onClick={() => props.onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** Cabecera de tarjeta de ajustes: icono, título y explicación. */
export function CardTitle({
  icon,
  title,
  hint,
  aside,
}: {
  icon: ReactNode;
  title: string;
  hint?: ReactNode;
  aside?: ReactNode;
}) {
  return (
    <div className="card-heading">
      <div className="card-heading-text">
        <h2>
          {icon}
          {title}
        </h2>
        {hint && <p className="muted">{hint}</p>}
      </div>
      {aside}
    </div>
  );
}

/** Barra de progreso; sin total conocido se muestra en movimiento. */
export function Progress({ done, total, label }: { done: number; total: number; label: string }) {
  const ratio = total > 0 ? Math.min(done / total, 1) : null;
  return (
    <div className="task-progress">
      <div className="task-progress-text">
        <span className="spinner" />
        <span>{label}</span>
        {ratio !== null && <span className="subtle">{formatPercent(ratio * 100)}</span>}
      </div>
      <div
        className="progress"
        data-indeterminate={ratio === null || undefined}
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={ratio === null ? undefined : Math.round(ratio * 100)}
      >
        <span style={{ width: `${Math.round((ratio ?? 0) * 100)}%` }} />
      </div>
    </div>
  );
}
