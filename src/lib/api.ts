// Envoltorio tipado de los comandos y eventos de Tauri (src-tauri/src/commands.rs).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  HardwareInfo,
  ModrinthHit,
  NewServerRequest,
  ServerConfig,
  ServerEventPayload,
  ServerStatsPayload,
  ServerSummary,
} from "./types";

export const api = {
  hardwareInfo: () => invoke<HardwareInfo>("hardware_info"),
  listServers: () => invoke<ServerSummary[]>("list_servers"),
  createServer: (request: NewServerRequest) => invoke<ServerConfig>("create_server", { request }),
  startServer: (id: string) => invoke<void>("start_server", { id }),
  stopServer: (id: string) => invoke<void>("stop_server", { id }),
  sendCommand: (id: string, command: string) => invoke<void>("send_command", { id, command }),
  searchModrinth: (query: string, kind: "modpack" | "mod" | "plugin" = "modpack") =>
    invoke<{ hits: ModrinthHit[]; total_hits: number }>("search_modrinth", {
      query: { query, kind, limit: 20, offset: 0 },
    }),
};

export const onServerEvent = (cb: (p: ServerEventPayload) => void): Promise<UnlistenFn> =>
  listen<ServerEventPayload>("server://event", (e) => cb(e.payload));

export const onServerStats = (cb: (p: ServerStatsPayload) => void): Promise<UnlistenFn> =>
  listen<ServerStatsPayload>("server://stats", (e) => cb(e.payload));
