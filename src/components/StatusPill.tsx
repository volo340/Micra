import { AlertTriangle } from "lucide-react";
import type { ServerStatus } from "../lib/types";

export const STATUS_LABEL: Record<ServerStatus, string> = {
  running: "En línea",
  starting: "Iniciando…",
  stopping: "Deteniendo…",
  stopped: "Apagado",
  crashed: "Se detuvo con un error",
};

export function StatusPill({ status }: { status: ServerStatus }) {
  const busy = status === "starting" || status === "stopping";
  return (
    <span className="status-pill" data-status={status}>
      {busy ? (
        <span className="spinner" />
      ) : status === "crashed" ? (
        <AlertTriangle size={14} />
      ) : (
        <span className="status-dot" />
      )}
      {STATUS_LABEL[status]}
    </span>
  );
}
