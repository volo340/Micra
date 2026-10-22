import { AlertTriangle } from "lucide-react";
import { useT } from "../i18n";
import type { ServerStatus } from "../lib/types";

export function StatusPill({ status }: { status: ServerStatus }) {
  const t = useT();
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
      {t.status[status]}
    </span>
  );
}
