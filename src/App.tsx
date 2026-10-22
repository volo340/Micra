import { useCallback, useEffect, useRef, useState } from "react";
import { Logo } from "./components/Logo";
import { Sidebar, type View } from "./components/Sidebar";
import { ToastProvider, useToast } from "./components/Toasts";
import { api, errorMessage } from "./lib/api";
import { useAppData } from "./lib/useAppData";
import type { ServerConfig } from "./lib/types";
import { Dashboard } from "./views/Dashboard";
import { Home } from "./views/Home";
import { Settings } from "./views/Settings";
import { Wizard } from "./views/wizard/Wizard";

export default function App() {
  return (
    <ToastProvider>
      <Shell />
    </ToastProvider>
  );
}

function Shell() {
  const toast = useToast();
  const data = useAppData(toast);
  const [view, setView] = useState<View>({ kind: "home" });
  const [busy, setBusy] = useState<Record<string, boolean>>({});
  const autoSelected = useRef(false);

  const onError = useCallback(
    (e: unknown) => toast({ tone: "error", title: "Algo ha fallado", message: errorMessage(e) }),
    [toast],
  );

  // Al abrir la app con servidores ya creados, mostrar directamente el primero.
  useEffect(() => {
    if (!autoSelected.current && data.servers?.length) {
      autoSelected.current = true;
      setView((v) => (v.kind === "home" ? { kind: "server", id: data.servers![0].config.id } : v));
    }
  }, [data.servers]);

  const run = async (id: string, action: () => Promise<void>) => {
    setBusy((b) => ({ ...b, [id]: true }));
    try {
      await action();
    } catch (e) {
      onError(e);
      data.refresh();
    } finally {
      setBusy((b) => ({ ...b, [id]: false }));
    }
  };

  const start = (id: string) =>
    run(id, async () => {
      data.resetStats(id);
      data.patch(id, (s) => ({ ...s, status: "starting" }));
      await api.startServer(id);
    });

  const stop = (id: string) => run(id, () => api.stopServer(id));

  const restart = (id: string) =>
    run(id, async () => {
      await api.stopServer(id);
      data.resetStats(id);
      data.patch(id, (s) => ({ ...s, status: "starting" }));
      await api.startServer(id);
    });

  const command = (id: string, cmd: string) => {
    data.echoCommand(id, cmd);
    api.sendCommand(id, cmd).catch(onError);
  };

  const onCreated = async (config: ServerConfig, startNow: boolean) => {
    await data.refresh();
    setView({ kind: "server", id: config.id });
    toast({ tone: "success", title: "Servidor creado", message: `«${config.name}» ya aparece en tu lista.` });
    if (startNow) start(config.id);
  };

  if (data.servers === null) {
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
        appInfo={data.appInfo}
        onCancel={() => setView(selected ? view : { kind: "home" })}
        onCreated={onCreated}
      />
    );
  } else if (view.kind === "settings") {
    content = <Settings hardware={data.hardware} appInfo={data.appInfo} onError={onError} />;
  } else if (selected) {
    const id = selected.config.id;
    content = (
      <Dashboard
        server={selected}
        lines={data.logs[id] ?? []}
        points={data.stats[id] ?? []}
        hardware={data.hardware}
        network={data.network}
        busy={Boolean(busy[id])}
        onStart={() => start(id)}
        onStop={() => stop(id)}
        onRestart={() => restart(id)}
        onCommand={(cmd) => command(id, cmd)}
        onClear={() => data.clearLogs(id)}
        onOpenFolder={() => api.openServerFolder(id).catch(onError)}
      />
    );
  } else {
    content = <Home onCreate={() => setView({ kind: "create" })} hasServers={data.servers.length > 0} />;
  }

  return (
    <div className="app">
      <Sidebar servers={data.servers} view={view} hardware={data.hardware} onNavigate={setView} />
      <main className="main">{content}</main>
    </div>
  );
}
