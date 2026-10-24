import { useCallback, useEffect, useRef, useState } from "react";
import { Logo } from "./components/Logo";
import { Sidebar, type ServerTab, type View } from "./components/Sidebar";
import { ToastProvider, useToast } from "./components/Toasts";
import { I18nProvider, useI18n } from "./i18n";
import { api, errorMessage } from "./lib/api";
import { useAppData } from "./lib/useAppData";
import type { AppUpdate, ServerConfig } from "./lib/types";
import { Home } from "./views/Home";
import { ServerView } from "./views/server/ServerView";
import { Settings } from "./views/Settings";
import { Wizard } from "./views/wizard/Wizard";

export default function App() {
  return (
    <I18nProvider>
      <ToastProvider>
        <Shell />
      </ToastProvider>
    </I18nProvider>
  );
}

function Shell() {
  const toast = useToast();
  const { t, setLang } = useI18n();
  const data = useAppData(toast);
  const [view, setView] = useState<View>({ kind: "home" });
  const [pending, setPending] = useState<Record<string, boolean>>({});
  const [update, setUpdate] = useState<AppUpdate | null>(null);
  const autoSelected = useRef(false);
  const mainRef = useRef<HTMLElement>(null);

  const onError = useCallback(
    (e: unknown) => toast({ tone: "error", title: t.common.somethingFailed, message: errorMessage(e) }),
    [toast, t],
  );

  // El idioma lo decide la app (el elegido en Ajustes o el del sistema).
  const language = data.appInfo?.language;
  useEffect(() => {
    if (language) setLang(language);
  }, [language, setLang]);

  // Buscar una versión nueva de Micra al abrir, si está activado.
  useEffect(() => {
    api
      .getSettings()
      .then((v) => (v.settings.updates.autoCheck ? api.checkAppUpdate() : null))
      .then((found) => found && setUpdate(found))
      .catch(() => {});
  }, []);

  // Al abrir la app con servidores ya creados, mostrar directamente el primero.
  useEffect(() => {
    if (!autoSelected.current && data.servers?.length) {
      autoSelected.current = true;
      setView((v) => (v.kind === "home" ? { kind: "server", id: data.servers![0].config.id, tab: "overview" } : v));
    }
  }, [data.servers]);

  // Cada pantalla y cada pestaña empiezan desde arriba.
  const screen = view.kind === "server" ? `${view.id}/${view.tab}` : view.kind;
  useEffect(() => {
    mainRef.current?.scrollTo({ top: 0 });
  }, [screen]);

  const run = async (id: string, action: () => Promise<void>) => {
    setPending((b) => ({ ...b, [id]: true }));
    try {
      await action();
    } catch (e) {
      onError(e);
      data.refresh();
    } finally {
      setPending((b) => ({ ...b, [id]: false }));
    }
  };

  const start = (id: string) =>
    run(id, async () => {
      data.resetStats(id);
      data.patch(id, (s) => ({ ...s, status: "starting" }));
      await api.startServer(id);
    });

  const stop = (id: string) => run(id, () => api.stopServer(id));

  // La vigilancia del servidor lo vuelve a encender en cuanto termina de apagarse.
  const restart = (id: string) =>
    run(id, async () => {
      await api.restartServer(id);
      data.resetStats(id);
    });

  const command = (id: string, cmd: string) => {
    data.echoCommand(id, cmd);
    api.sendCommand(id, cmd).catch(onError);
  };

  // Del servidor borrado se pasa al primero que quede (o al inicio).
  const onDeleted = (id: string, name: string) => {
    const rest = (data.servers ?? []).filter((s) => s.config.id !== id);
    data.forget(id);
    setView(rest.length ? { kind: "server", id: rest[0].config.id, tab: "overview" } : { kind: "home" });
    toast({ tone: "success", title: t.server.deleted(name) });
    data.refresh();
  };

  const onCreated = async (config: ServerConfig, startNow: boolean) => {
    await data.refresh();
    setView({ kind: "server", id: config.id, tab: "overview" });
    toast({ tone: "success", title: t.server.created, message: t.server.createdMessage(config.name) });
    if (startNow) start(config.id);
  };

  if (data.servers === null || data.appInfo === undefined) {
    return (
      <div className="splash">
        <Logo size={72} className="splash-logo" />
      </div>
    );
  }

  const selected = view.kind === "server" ? data.servers.find((s) => s.config.id === view.id) : undefined;

  let content;
  if (view.kind === "create") {
    content = (
      <Wizard
        hardware={data.hardware}
        usedPorts={data.servers.map((s) => s.config.port)}
        onCancel={() => setView(selected ? view : { kind: "home" })}
        onCreated={onCreated}
      />
    );
  } else if (view.kind === "settings") {
    content = (
      <Settings
        hardware={data.hardware}
        appInfo={data.appInfo}
        update={update}
        onUpdateFound={setUpdate}
        onAppInfoChanged={data.refreshAppInfo}
        onError={onError}
      />
    );
  } else if (selected && view.kind === "server") {
    const id = selected.config.id;
    content = (
      <ServerView
        server={selected}
        data={data}
        tab={view.tab}
        pending={Boolean(pending[id])}
        onTab={(tab: ServerTab) => setView({ kind: "server", id, tab })}
        onStart={() => start(id)}
        onStop={() => stop(id)}
        onRestart={() => restart(id)}
        onCommand={(cmd) => command(id, cmd)}
        onOpenAppSettings={() => setView({ kind: "settings" })}
        onDeleted={() => onDeleted(id, selected.config.name)}
        onError={onError}
      />
    );
  } else {
    content = <Home onCreate={() => setView({ kind: "create" })} hasServers={data.servers.length > 0} />;
  }

  return (
    <div className="app">
      <Sidebar
        servers={data.servers}
        view={view}
        hardware={data.hardware}
        update={update?.version ?? null}
        onNavigate={setView}
      />
      <main className="main" ref={mainRef}>
        {content}
      </main>
    </div>
  );
}
