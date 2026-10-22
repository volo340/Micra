import { createContext, useCallback, useContext, useState, type ReactNode } from "react";
import { AlertCircle, CheckCircle2, Info, X } from "lucide-react";

type Tone = "success" | "error" | "info";

interface Toast {
  id: number;
  tone: Tone;
  title: string;
  message?: string;
}

type Push = (toast: Omit<Toast, "id">) => void;

const ToastContext = createContext<Push>(() => {});
let seq = 0;

const ICONS = { success: CheckCircle2, error: AlertCircle, info: Info };

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);

  const dismiss = useCallback((id: number) => setToasts((ts) => ts.filter((t) => t.id !== id)), []);

  const push = useCallback<Push>(
    (toast) => {
      const id = ++seq;
      setToasts((ts) => [...ts.slice(-3), { ...toast, id }]);
      window.setTimeout(() => dismiss(id), toast.tone === "error" ? 9000 : 4500);
    },
    [dismiss],
  );

  return (
    <ToastContext.Provider value={push}>
      {children}
      <div className="toast-stack" role="status" aria-live="polite">
        {toasts.map((t) => {
          const Icon = ICONS[t.tone];
          return (
            <div key={t.id} className="toast" data-tone={t.tone}>
              <Icon size={18} className="toast-icon" />
              <div className="toast-body">
                <div className="toast-title">{t.title}</div>
                {t.message && <div className="toast-message">{t.message}</div>}
              </div>
              <button className="btn btn-ghost btn-icon btn-sm" onClick={() => dismiss(t.id)} aria-label="Cerrar aviso">
                <X size={14} />
              </button>
            </div>
          );
        })}
      </div>
    </ToastContext.Provider>
  );
}

export const useToast = () => useContext(ToastContext);
