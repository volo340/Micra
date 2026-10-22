import { useCallback, useEffect, useRef, useState } from "react";
import { useT } from "../i18n";
import { api, errorMessage, onAppError, onServerEvent, onServerLive, onServerStats } from "./api";
import { commandLine, parseLogLine, type LogLine } from "./logs";
import type { AppInfo, HardwareInfo, NetworkInfo, RejectReason, ServerStatus, ServerSummary } from "./types";

export interface StatsPoint {
  cpu: number;
  memMb: number;
}

/** Alguien intentó entrar y el servidor no le dejó. */
export interface Rejection {
  name: string;
  reason: RejectReason;
  at: number;
}

const MAX_LOG_LINES = 2500;
const MAX_POINTS = 60;
const FLUSH_MS = 80;
const MAX_REJECTIONS = 8;
/** No repetir el aviso del mismo jugador rechazado antes de este tiempo. */
const REJECTION_TOAST_MS = 60_000;

export interface Notice {
  tone: "success" | "error" | "info";
  title: string;
  message?: string;
}

/** Estado global: servidores, consola, estadísticas en vivo y avisos. */
export function useAppData(notify: (notice: Notice) => void) {
  const t = useT();
  const [servers, setServers] = useState<ServerSummary[] | null>(null);
  const [logs, setLogs] = useState<Record<string, LogLine[]>>({});
  const [stats, setStats] = useState<Record<string, StatsPoint[]>>({});
  const [tps, setTps] = useState<Record<string, number | null>>({});
  const [rejections, setRejections] = useState<Record<string, Rejection[]>>({});
  const [hardware, setHardware] = useState<HardwareInfo | null>(null);
  const [network, setNetwork] = useState<NetworkInfo | null>(null);
  // `undefined` mientras carga; `null` si no se pudo leer.
  const [appInfo, setAppInfo] = useState<AppInfo | null | undefined>(undefined);

  // Un servidor con mods escribe miles de líneas al arrancar: se agrupan en
  // lotes para no repintar la interfaz por cada una.
  const pending = useRef<Record<string, LogLine[]>>({});
  const timer = useRef<number | null>(null);
  const notifyRef = useRef(notify);
  notifyRef.current = notify;
  const tRef = useRef(t);
  tRef.current = t;
  const serversRef = useRef<ServerSummary[] | null>(null);
  serversRef.current = servers;
  const lastRejectionToast = useRef<Record<string, number>>({});
  // Una lista pedida antes de un evento puede llegar después de él: se cuentan
  // los eventos de cada servidor para no pisar lo que ya se sabe.
  const eventCount = useRef<Record<string, number>>({});
  const refreshSeq = useRef(0);
  const appliedSeq = useRef(0);
  const bump = (id: string) => {
    eventCount.current[id] = (eventCount.current[id] ?? 0) + 1;
  };

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
    const seq = ++refreshSeq.current;
    const before = { ...eventCount.current };
    try {
      const list = await api.listServers();
      // Ya respondió una petición más nueva.
      if (seq < appliedSeq.current) return;
      appliedSeq.current = seq;
      setServers((prev) =>
        list.map((fresh) => {
          const id = fresh.config.id;
          const known = prev?.find((s) => s.config.id === id);
          if (!known || eventCount.current[id] === before[id]) return fresh;
          return { ...fresh, status: known.status, players: known.players, live: known.live };
        }),
      );
    } catch (e) {
      notifyRef.current({ tone: "error", title: tRef.current.server.listError, message: errorMessage(e) });
      setServers((s) => s ?? []);
    }
  }, []);

  const refreshAppInfo = useCallback(async () => {
    try {
      setAppInfo(await api.appInfo());
    } catch {
      setAppInfo((info) => info ?? null);
    }
  }, []);

  const patch = useCallback((id: string, change: (s: ServerSummary) => ServerSummary) => {
    setServers((prev) => prev?.map((s) => (s.config.id === id ? change(s) : s)) ?? prev);
  }, []);

  const nameOf = (id: string) => serversRef.current?.find((s) => s.config.id === id)?.config.name ?? id;

  useEffect(() => {
    refresh();
    refreshAppInfo();
    api
      .hardwareInfo()
      .then(setHardware)
      .catch(() => {});
    api
      .networkInfo()
      .then(setNetwork)
      .catch(() => {});

    const subscriptions = [
      onServerEvent(({ serverId, event }) => {
        switch (event.type) {
          case "log":
            pushLines(serverId, [parseLogLine(event.line, event.stream)]);
            break;
          case "status":
            bump(serverId);
            patch(serverId, (s) => ({ ...s, status: event.status as ServerStatus }));
            if (event.status === "stopped" || event.status === "crashed") {
              patch(serverId, (s) => ({ ...s, players: [], startedAt: null }));
              setTps((prev) => ({ ...prev, [serverId]: null }));
            }
            // Al arrancar o apagarse cambian la hora de inicio y a veces la
            // configuración (el dueño pendiente, por ejemplo).
            if (event.status === "running" || event.status === "stopped" || event.status === "crashed") {
              refresh();
            }
            if (event.status === "crashed") {
              notifyRef.current({
                tone: "error",
                title: tRef.current.server.crashed(nameOf(serverId)),
                message: tRef.current.server.crashedHint,
              });
            }
            break;
          case "playerJoined":
            bump(serverId);
            patch(serverId, (s) => ({ ...s, players: [...new Set([...s.players, event.name])].sort() }));
            setRejections((prev) => ({
              ...prev,
              [serverId]: (prev[serverId] ?? []).filter((r) => r.name !== event.name),
            }));
            break;
          case "playerLeft":
            bump(serverId);
            patch(serverId, (s) => ({ ...s, players: s.players.filter((p) => p !== event.name) }));
            break;
          case "joinRejected": {
            const entry: Rejection = { name: event.name, reason: event.reason, at: Date.now() };
            setRejections((prev) => ({
              ...prev,
              [serverId]: [entry, ...(prev[serverId] ?? []).filter((r) => r.name !== event.name)].slice(
                0,
                MAX_REJECTIONS,
              ),
            }));
            const key = `${serverId}/${event.name}`;
            if (Date.now() - (lastRejectionToast.current[key] ?? 0) > REJECTION_TOAST_MS) {
              lastRejectionToast.current[key] = Date.now();
              const tr = tRef.current;
              notifyRef.current({
                tone: "info",
                title: tr.server.rejected(event.name, nameOf(serverId)),
                message: event.reason === "banned" ? tr.server.rejectedBanned : tr.server.rejectedWhitelist,
              });
            }
            break;
          }
          case "exited":
            break;
        }
      }),
      onServerStats(({ serverId, sample, tps: value }) => {
        setStats((prev) => ({
          ...prev,
          [serverId]: [...(prev[serverId] ?? []), { cpu: sample.cpuPercent, memMb: sample.memoryMb }].slice(
            -MAX_POINTS,
          ),
        }));
        setTps((prev) => (prev[serverId] === value ? prev : { ...prev, [serverId]: value }));
      }),
      onServerLive(({ serverId, live }) => {
        bump(serverId);
        patch(serverId, (s) => ({ ...s, live }));
      }),
      onAppError(({ title, message }) => notifyRef.current({ tone: "error", title, message })),
    ];
    return () => {
      subscriptions.forEach((p) => p.then((off) => off()));
      if (timer.current !== null) window.clearTimeout(timer.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refresh, refreshAppInfo, patch, pushLines]);

  const echoCommand = useCallback((id: string, command: string) => pushLines(id, [commandLine(command)]), [pushLines]);

  const clearLogs = useCallback((id: string) => setLogs((prev) => ({ ...prev, [id]: [] })), []);

  const resetStats = useCallback((id: string) => setStats((prev) => ({ ...prev, [id]: [] })), []);

  const dismissRejection = useCallback(
    (id: string, name: string) =>
      setRejections((prev) => ({ ...prev, [id]: (prev[id] ?? []).filter((r) => r.name !== name) })),
    [],
  );

  return {
    servers,
    logs,
    stats,
    tps,
    rejections,
    hardware,
    network,
    appInfo,
    refresh,
    refreshAppInfo,
    patch,
    echoCommand,
    clearLogs,
    resetStats,
    dismissRejection,
  };
}

export type AppData = ReturnType<typeof useAppData>;
