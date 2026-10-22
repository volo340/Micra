import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorMessage, onServerEvent, onServerStats } from "./api";
import { commandLine, parseLogLine, type LogLine } from "./logs";
import type { AppInfo, HardwareInfo, NetworkInfo, ServerStatus, ServerSummary } from "./types";

export interface StatsPoint {
  cpu: number;
  memMb: number;
}

const MAX_LOG_LINES = 2500;
const MAX_POINTS = 60;
const FLUSH_MS = 80;

export interface Notice {
  tone: "success" | "error" | "info";
  title: string;
  message?: string;
}

/** Estado global: servidores, consola y estadísticas en vivo. */
export function useAppData(notify: (notice: Notice) => void) {
  const [servers, setServers] = useState<ServerSummary[] | null>(null);
  const [logs, setLogs] = useState<Record<string, LogLine[]>>({});
  const [stats, setStats] = useState<Record<string, StatsPoint[]>>({});
  const [hardware, setHardware] = useState<HardwareInfo | null>(null);
  const [network, setNetwork] = useState<NetworkInfo | null>(null);
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);

  // Un servidor con mods escribe miles de líneas al arrancar: se agrupan en
  // lotes para no repintar la interfaz por cada una.
  const pending = useRef<Record<string, LogLine[]>>({});
  const timer = useRef<number | null>(null);
  const notifyRef = useRef(notify);
  notifyRef.current = notify;
  const serversRef = useRef<ServerSummary[] | null>(null);
  serversRef.current = servers;

  const flush = useCallback(() => {
    timer.current = null;
    const batch = pending.current;
    pending.current = {};
    setLogs((prev) => {
      const next = { ...prev };
      for (const [id, lines] of Object.entries(batch)) {
        next[id] = [...(prev[id] ?? []), ...lines].slice(-MAX_LOG_LINES);
      }
      return next;
    });
  }, []);

  const pushLines = useCallback(
    (id: string, lines: LogLine[]) => {
      (pending.current[id] ??= []).push(...lines);
      if (timer.current === null) timer.current = window.setTimeout(flush, FLUSH_MS);
    },
    [flush],
  );

  const refresh = useCallback(async () => {
    try {
      setServers(await api.listServers());
    } catch (e) {
      notifyRef.current({ tone: "error", title: "No se pudo leer la lista de servidores", message: errorMessage(e) });
      setServers((s) => s ?? []);
    }
  }, []);

  const patch = useCallback((id: string, change: (s: ServerSummary) => ServerSummary) => {
    setServers((prev) => prev?.map((s) => (s.config.id === id ? change(s) : s)) ?? prev);
  }, []);

  useEffect(() => {
    refresh();
    api
      .hardwareInfo()
      .then(setHardware)
      .catch(() => {});
    api
      .networkInfo()
      .then(setNetwork)
      .catch(() => {});
    api
      .appInfo()
      .then(setAppInfo)
      .catch(() => {});

    const subscriptions = [
      onServerEvent(({ serverId, event }) => {
        switch (event.type) {
          case "log":
            pushLines(serverId, [parseLogLine(event.line, event.stream)]);
            break;
          case "status":
            patch(serverId, (s) => ({ ...s, status: event.status as ServerStatus }));
            if (event.status === "stopped" || event.status === "crashed") {
              patch(serverId, (s) => ({ ...s, players: [] }));
            }
            if (event.status === "crashed") {
              const name = serversRef.current?.find((s) => s.config.id === serverId)?.config.name ?? serverId;
              notifyRef.current({
                tone: "error",
                title: `«${name}» se ha detenido con un error`,
                message: "Revisa la consola para ver qué ha pasado.",
              });
            }
            break;
          case "playerJoined":
            patch(serverId, (s) => ({ ...s, players: [...new Set([...s.players, event.name])].sort() }));
            break;
          case "playerLeft":
            patch(serverId, (s) => ({ ...s, players: s.players.filter((p) => p !== event.name) }));
            break;
          case "exited":
            break;
        }
      }),
      onServerStats(({ serverId, sample }) => {
        setStats((prev) => ({
          ...prev,
          [serverId]: [...(prev[serverId] ?? []), { cpu: sample.cpuPercent, memMb: sample.memoryMb }].slice(
            -MAX_POINTS,
          ),
        }));
      }),
    ];
    return () => {
      subscriptions.forEach((p) => p.then((off) => off()));
      if (timer.current !== null) window.clearTimeout(timer.current);
    };
  }, [refresh, patch, pushLines]);

  const echoCommand = useCallback((id: string, command: string) => pushLines(id, [commandLine(command)]), [pushLines]);

  const clearLogs = useCallback((id: string) => setLogs((prev) => ({ ...prev, [id]: [] })), []);

  const resetStats = useCallback((id: string) => setStats((prev) => ({ ...prev, [id]: [] })), []);

  return {
    servers,
    logs,
    stats,
    hardware,
    network,
    appInfo,
    refresh,
    patch,
    echoCommand,
    clearLogs,
    resetStats,
  };
}
