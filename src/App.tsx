import { useEffect, useRef, useState } from "react";
import { api, onServerEvent, onServerStats } from "./lib/api";
import type { ServerStatsPayload, ServerStatus, ServerSummary } from "./lib/types";

const MAX_LINES = 2000;

const STATUS_LABEL: Record<ServerStatus, string> = {
  starting: "Iniciando",
  running: "En línea",
  stopping: "Deteniendo",
  stopped: "Apagado",
  crashed: "Se cerró con error",
};

export default function App() {
  const [servers, setServers] = useState<ServerSummary[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [logs, setLogs] = useState<Record<string, string[]>>({});
  const [stats, setStats] = useState<Record<string, ServerStatsPayload>>({});
  const [error, setError] = useState<string | null>(null);

  const refresh = () =>
    api
      .listServers()
      .then((list) => {
        setServers(list);
        setSelectedId((id) => id ?? list[0]?.config.id ?? null);
      })
      .catch((e) => setError(String(e)));

  useEffect(() => {
    refresh();
    const unlisten = [
      onServerEvent(({ serverId, event }) => {
        if (event.type === "log") {
          setLogs((prev) => {
            const lines = [...(prev[serverId] ?? []), event.line];
            return { ...prev, [serverId]: lines.slice(-MAX_LINES) };
          });
        } else {
          refresh();
        }
      }),
      onServerStats((p) => setStats((prev) => ({ ...prev, [p.serverId]: p }))),
    ];
    return () => unlisten.forEach((p) => p.then((off) => off()));
  }, []);

  const selected = servers.find((s) => s.config.id === selectedId) ?? null;

  return (
    <div className="layout">
      <aside className="sidebar">
        <h1 className="brand">Micra</h1>
        <button className="primary" onClick={() => alert("El asistente de creación llega en la siguiente fase")}>
          + Nuevo servidor
        </button>
        <nav>
          {servers.map((s) => (
            <button
              key={s.config.id}
              className={`server-item ${s.config.id === selectedId ? "active" : ""}`}
              onClick={() => setSelectedId(s.config.id)}
            >
              <span className={`dot ${s.status}`} />
              {s.config.name}
            </button>
          ))}
          {servers.length === 0 && <p className="muted">Todavía no hay servidores.</p>}
        </nav>
      </aside>

      <main className="content">
        {error && <div className="error" onClick={() => setError(null)}>{error}</div>}
        {selected ? (
          <Dashboard
            server={selected}
            lines={logs[selected.config.id] ?? []}
            stats={stats[selected.config.id]}
            onError={(e) => setError(String(e))}
          />
        ) : (
          <div className="empty">Crea tu primer servidor para empezar.</div>
        )}
      </main>
    </div>
  );
}

function Dashboard(props: {
  server: ServerSummary;
  lines: string[];
  stats?: ServerStatsPayload;
  onError: (e: unknown) => void;
}) {
  const { server, lines, stats, onError } = props;
  const { config, status, players } = server;
  const alive = status === "starting" || status === "running" || status === "stopping";
  const [command, setCommand] = useState("");
  const consoleRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    consoleRef.current?.scrollTo({ top: consoleRef.current.scrollHeight });
  }, [lines.length]);

  const toggle = () =>
    (alive ? api.stopServer(config.id) : api.startServer(config.id)).catch(onError);

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!command.trim()) return;
    api.sendCommand(config.id, command).catch(onError);
    setCommand("");
  };

  return (
    <section className="dashboard">
      <header className="dash-header">
        <div>
          <h2>{config.name}</h2>
          <p className="muted">
            Minecraft {config.mcVersion} · {config.loader.kind} · Java {config.javaMajor} · puerto {config.port}
          </p>
        </div>
        <div className="actions">
          <span className={`badge ${status}`}>{STATUS_LABEL[status]}</span>
          <button className={alive ? "danger" : "primary"} onClick={toggle} disabled={status === "stopping"}>
            {alive ? "Apagar" : "Encender"}
          </button>
        </div>
      </header>

      <div className="tiles">
        <Tile label="CPU" value={stats ? `${stats.sample.cpuPercent.toFixed(0)} %` : "—"} />
        <Tile
          label="RAM"
          value={stats ? `${(stats.sample.memoryMb / 1024).toFixed(1)} / ${(config.memory.maxMb / 1024).toFixed(1)} GB` : "—"}
        />
        <Tile label="Jugadores" value={String(players.length)} />
      </div>

      <div className="panels">
        <div className="console">
          <pre ref={consoleRef}>{lines.join("\n")}</pre>
          <form onSubmit={submit}>
            <input
              value={command}
              onChange={(e) => setCommand(e.target.value)}
              placeholder={alive ? "Escribe un comando (p. ej. say hola)" : "El servidor está apagado"}
              disabled={!alive}
            />
          </form>
        </div>
        <div className="players">
          <h3>Conectados</h3>
          {players.length ? players.map((p) => <div key={p}>{p}</div>) : <p className="muted">Nadie por ahora.</p>}
        </div>
      </div>
    </section>
  );
}

function Tile({ label, value }: { label: string; value: string }) {
  return (
    <div className="tile">
      <span className="muted">{label}</span>
      <strong>{value}</strong>
    </div>
  );
}
