import { useCallback, useEffect, useState, type ReactNode } from "react";
import { ConnectCard } from "../../components/ConnectCard";
import { Console } from "../../components/Console";
import { PlayersPanel } from "../../components/PlayersPanel";
import type { ServerTab } from "../../components/Sidebar";
import { CpuTile, MemoryTile, PlayersTile, TpsTile } from "../../components/Tiles";
import { useToast } from "../../components/Toasts";
import { Banner } from "../../components/ui";
import { useT } from "../../i18n";
import { api } from "../../lib/api";
import type { ServerSummary } from "../../lib/types";
import type { AppData } from "../../lib/useAppData";
import { CrashCard } from "./CrashCard";

interface Props {
  server: ServerSummary;
  data: AppData;
  onTab: (tab: ServerTab) => void;
  onStart: () => void;
  onCommand: (command: string) => void;
  onError: (e: unknown) => void;
}

/** Pestaña Panel: por qué falló, avisos, cifras en vivo, consola, direcciones y jugadores. */
export function Overview({ server, data, onTab, onStart, onCommand, onError }: Props) {
  const t = useT();
  const toast = useToast();
  const { config, status, players, live } = server;
  const id = config.id;
  const running = status === "running";
  const alive = running || status === "starting";
  const [properties, setProperties] = useState<Record<string, string> | null>(null);

  const loadProperties = useCallback(
    () =>
      api
        .getProperties(id)
        .then((v) => setProperties(v.values))
        .catch(() => setProperties(null)),
    [id],
  );

  // Minecraft reescribe server.properties al arrancar: se relee entonces.
  useEffect(() => {
    loadProperties();
  }, [loadProperties, running]);

  const secure = async (key: "online-mode" | "white-list") => {
    try {
      const changes: Record<string, string> =
        key === "white-list" ? { "white-list": "true", "enforce-whitelist": "true" } : { "online-mode": "true" };
      await api.saveProperties(id, changes);
      // La lista blanca se puede activar en caliente; la comprobación de cuentas, al reiniciar.
      if (running && key === "white-list") await api.sendCommand(id, "whitelist on");
      const later = running && key === "online-mode";
      toast({ tone: "success", title: later ? t.overview.securedAfterRestart : t.overview.secured });
      loadProperties();
    } catch (e) {
      onError(e);
    }
  };

  const letIn = async (name: string) => {
    try {
      await api.playerAction(id, { kind: "whitelistAdd" }, name);
      data.dismissRejection(id, name);
      toast({ tone: "success", title: t.playersPanel.letInDone(name) });
    } catch (e) {
      onError(e);
    }
  };

  const banners: ReactNode[] = [];
  const view = (tab: ServerTab) => (
    <button className="btn btn-secondary btn-sm" onClick={() => onTab(tab)}>
      {t.common.view}
    </button>
  );
  if (properties?.["online-mode"] === "false") {
    banners.push(
      <Banner
        key="online"
        tone="critical"
        action={
          <button className="btn btn-secondary btn-sm" onClick={() => secure("online-mode")}>
            {t.common.enable}
          </button>
        }
      >
        {t.overview.onlineModeOff}
      </Banner>,
    );
  }
  if (properties?.["white-list"] === "false") {
    banners.push(
      <Banner
        key="whitelist"
        tone="warning"
        action={
          <button className="btn btn-secondary btn-sm" onClick={() => secure("white-list")}>
            {t.common.enable}
          </button>
        }
      >
        {t.overview.whitelistOff}
      </Banner>,
    );
  }
  if (live.lastBackupError) {
    banners.push(
      <Banner key="backup" tone="critical" action={view("backups")}>
        {t.overview.backupFailed(live.lastBackupError)}
      </Banner>,
    );
  }
  if (live.mapNeedsAccept) {
    banners.push(
      <Banner
        key="map"
        tone="warning"
        action={
          <button
            className="btn btn-secondary btn-sm"
            onClick={() =>
              api
                .acceptMapDownload(id)
                .then(() => data.refresh())
                .catch(onError)
            }
          >
            {t.overview.mapAccept}
          </button>
        }
      >
        {t.overview.mapNeedsAccept}
      </Banner>,
    );
  }
  if (live.packUpdate) {
    banners.push(
      <Banner key="pack" action={view("mods")}>
        {t.overview.packUpdate(live.packUpdate.latest)}
      </Banner>,
    );
  } else if (live.modUpdates > 0) {
    banners.push(
      <Banner key="mods" action={view("mods")}>
        {t.overview.modUpdates(live.modUpdates)}
      </Banner>,
    );
  }
  if (alive && live.sandbox?.kind === "unavailable") {
    banners.push(
      <Banner key="sandbox" tone="warning">
        {t.overview.sandboxUnavailable(live.sandbox.reason)}
      </Banner>,
    );
  }

  return (
    <>
      {live.crash && <CrashCard server={server} onTab={onTab} onStart={onStart} onError={onError} />}
      {banners.length > 0 && <div className="banners">{banners}</div>}

      <div className="tiles">
        <CpuTile
          points={data.stats[id] ?? []}
          live={alive}
          cores={config.cpu.cores}
          logicalCpus={data.hardware?.logicalCpus ?? 0}
        />
        <MemoryTile points={data.stats[id] ?? []} live={alive} maxMb={config.memory.maxMb} />
        <TpsTile tps={data.tps[id] ?? live.tps} live={running} />
        <PlayersTile players={players} live={running} />
      </div>

      <div className="workspace">
        <Console lines={data.logs[id] ?? []} live={alive} onCommand={onCommand} onClear={() => data.clearLogs(id)} />
        <div className="side-column">
          <ConnectCard server={server} network={data.network} onConfigure={() => onTab("settings")} />
          <PlayersPanel
            players={players}
            live={running}
            rejections={data.rejections[id] ?? []}
            onCommand={onCommand}
            onLetIn={letIn}
            onDismiss={(name) => data.dismissRejection(id, name)}
          />
        </div>
      </div>
    </>
  );
}
