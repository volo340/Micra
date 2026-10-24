import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { AlertTriangle } from "lucide-react";
import { useT } from "../i18n";

export interface ConfirmOptions {
  title: string;
  message?: ReactNode;
  confirmLabel: string;
  danger?: boolean;
  /** Campo de texto opcional (por ejemplo, el motivo de un baneo). */
  input?: { label: string; placeholder?: string };
  /** Casilla opcional (por ejemplo, borrar también las copias); cada cambio llega a `onChange`. */
  check?: { label: string; hint?: string; defaultChecked?: boolean; onChange: (checked: boolean) => void };
}

function ConfirmDialog(props: { options: ConfirmOptions; onConfirm: (value: string) => void; onCancel: () => void }) {
  const t = useT();
  const { options, onConfirm, onCancel } = props;
  const [value, setValue] = useState("");
  const [checked, setChecked] = useState(Boolean(options.check?.defaultChecked));
  const confirmRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onCancel();
    window.addEventListener("keydown", onKey);
    if (!options.input) confirmRef.current?.focus();
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel, options.input]);

  return (
    <div className="dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onCancel()}>
      <div className="dialog" role="alertdialog" aria-modal="true" aria-labelledby="dialog-title">
        <div className="dialog-head">
          {options.danger && (
            <span className="dialog-icon">
              <AlertTriangle size={18} />
            </span>
          )}
          <h2 id="dialog-title">{options.title}</h2>
        </div>
        {options.message && <div className="dialog-body">{options.message}</div>}
        {options.input && (
          <label className="field">
            <span className="field-label">{options.input.label}</span>
            <input
              className="input"
              autoFocus
              value={value}
              maxLength={200}
              placeholder={options.input.placeholder}
              onChange={(e) => setValue(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && onConfirm(value.trim())}
            />
          </label>
        )}
        {options.check && (
          <label className="check check-sm">
            <input
              type="checkbox"
              checked={checked}
              onChange={(e) => {
                setChecked(e.target.checked);
                options.check?.onChange(e.target.checked);
              }}
            />
            <span className="check-text">
              <span>{options.check.label}</span>
              {options.check.hint && <span className="field-hint">{options.check.hint}</span>}
            </span>
          </label>
        )}
        <div className="dialog-actions">
          <button className="btn btn-ghost" onClick={onCancel}>
            {t.common.cancel}
          </button>
          <button
            ref={confirmRef}
            className={options.danger ? "btn btn-danger" : "btn btn-primary"}
            onClick={() => onConfirm(value.trim())}
          >
            {options.confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

/**
 * Diálogo de confirmación como promesa: `await confirm({...})` devuelve el
 * texto escrito (vacío si no hay campo) o `null` si la persona cancela.
 */
export function useConfirm() {
  const [state, setState] = useState<{ options: ConfirmOptions; resolve: (v: string | null) => void } | null>(null);

  const confirm = useCallback(
    (options: ConfirmOptions) => new Promise<string | null>((resolve) => setState({ options, resolve })),
    [],
  );

  const close = useCallback(
    (value: string | null) => {
      state?.resolve(value);
      setState(null);
    },
    [state],
  );
  const cancel = useCallback(() => close(null), [close]);

  const dialog = state ? <ConfirmDialog options={state.options} onConfirm={close} onCancel={cancel} /> : null;
  return [dialog, confirm] as const;
}
