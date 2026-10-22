import { api } from "./api";
import type { ServerConfig, ServerSettings, SettingsSaved } from "./types";

/** Los ajustes editables de un servidor tal como están guardados. */
export function settingsOf(config: ServerConfig): ServerSettings {
  return {
    memory: config.memory,
    cpu: config.cpu,
    port: config.port,
    useAikarFlags: config.useAikarFlags,
    backups: config.backups,
    schedule: config.schedule,
    network: config.network,
    security: config.security,
    mapPort: config.map.port,
  };
}

/** Guarda solo lo que cambia; el resto se envía como está. */
export function saveServerSettings(config: ServerConfig, patch: Partial<ServerSettings>): Promise<SettingsSaved> {
  return api.updateServerSettings(config.id, { ...settingsOf(config), ...patch });
}

/** Igualdad profunda para saber si un borrador tiene cambios. */
export const sameValue = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

export const sleep = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));

/** Nombres de cuenta de Minecraft: 1–16 letras, números o _. */
export const validPlayerName = (name: string) => /^[A-Za-z0-9_]{1,16}$/.test(name);
