// Backend simulado para `npm run dev` en un navegador normal (sin Tauri).
// Sirve para diseñar y probar la interfaz; nunca se incluye en la app final.
//
// Parámetros de la URL:
//   ?state=empty    sin servidores (pantalla de bienvenida)
//   ?state=running  el primer servidor arranca solo, con jugadores y gráficas
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { NewServerRequest, ServerConfig, ServerStatus, ServerSummary } from "../lib/types";

const params = new URLSearchParams(location.search);
const scenario = params.get("state");

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const DATA_DIR = "/home/steve/.local/share/micra";

let servers: ServerSummary[] =
  scenario === "empty"
    ? []
    : [
        {
          config: {
            id: "survival-con-amigos",
            name: "Survival con amigos",
            mcVersion: "1.21.4",
            loader: { kind: "vanilla" },
            javaMajor: 21,
            memory: { minMb: 2048, maxMb: 4096 },
            cpu: { cores: null },
            port: 25565,
            eulaAccepted: true,
          },
          status: "stopped",
          players: [],
          dir: `${DATA_DIR}/servers/survival-con-amigos`,
        },
        {
          config: {
            id: "aventura-con-mods",
            name: "Aventura con mods",
            mcVersion: "1.20.1",
            loader: { kind: "fabric", loaderVersion: "0.16.9" },
            javaMajor: 17,
            memory: { minMb: 2048, maxMb: 6144 },
            cpu: { cores: [0, 1, 2, 3, 4, 5] },
            port: 25566,
            eulaAccepted: true,
          },
          status: "stopped",
          players: [],
          dir: `${DATA_DIR}/servers/aventura-con-mods`,
        },
      ];

const timers = new Map<string, number[]>();

const clock = () => new Date().toTimeString().slice(0, 8);
const log = (serverId: string, message: string, level = "INFO", thread = "Server thread") =>
  emit("server://event", {
    serverId,
    event: { type: "log", stream: "stdout", line: `[${clock()}] [${thread}/${level}]: ${message}` },
  });

function setStatus(id: string, status: ServerStatus) {
  servers = servers.map((s) => (s.config.id === id ? { ...s, status } : s));
  emit("server://event", { serverId: id, event: { type: "status", status } });
}

function setPlayers(id: string, change: (p: string[]) => string[]) {
  servers = servers.map((s) => (s.config.id === id ? { ...s, players: change(s.players) } : s));
}

function join(id: string, name: string) {
  setPlayers(id, (p) => [...p, name]);
  log(
    id,
    `${name}[/192.168.1.${20 + name.length}:5${name.length}812] logged in with entity id 1${name.length}2 at (12.5, 64.0, -30.5)`,
  );
  log(id, `${name} joined the game`);
  emit("server://event", { serverId: id, event: { type: "playerJoined", name } });
}

function leave(id: string, name: string) {
  setPlayers(id, (p) => p.filter((x) => x !== name));
  log(id, `${name} lost connection: Disconnected`);
  log(id, `${name} left the game`);
  emit("server://event", { serverId: id, event: { type: "playerLeft", name } });
}

const STARTUP = [
  "Starting minecraft server version 1.21.4",
  "Loading properties",
  "Default game type: SURVIVAL",
  "Generating keypair",
  "Starting Minecraft server on *:25565",
  "Using epoll channel type",
  'Preparing level "world"',
  "Preparing start region for dimension minecraft:overworld",
  "Preparing spawn area: 0%",
  "Preparing spawn area: 41%",
  "Preparing spawn area: 83%",
  "Time elapsed: 2843 ms",
];

function startStats(id: string, history = 0) {
  let cpu = 18;
  let mem = 1650;
  const point = () => {
    const players = servers.find((s) => s.config.id === id)?.players.length ?? 0;
    cpu = Math.max(3, Math.min(92, cpu + (Math.random() - 0.5) * 9 + (players * 6 - cpu) * 0.08));
    mem = Math.max(1200, Math.min(3900, mem + (Math.random() - 0.4) * 45));
    emit("server://stats", { serverId: id, sample: { cpuPercent: cpu, memoryMb: mem }, playerCount: players });
  };
  for (let i = 0; i < history; i++) point();
  return window.setInterval(point, 1000);
}

async function start(id: string, fast = false) {
  setStatus(id, "starting");
  const own: number[] = [];
  timers.set(id, own);
  for (const line of STARTUP) {
    log(id, line, "INFO", line.startsWith("Preparing") ? "Worker-Main-2" : "Server thread");
    await sleep(fast ? 5 : 120);
  }
  log(id, "Can't keep up! Is the server overloaded? Running 2044ms or 40 ticks behind", "WARN");
  log(id, 'Done (3.214s)! For help, type "help"');
  setStatus(id, "running");
  own.push(startStats(id, fast ? 48 : 0));

  const roster = ["Steve", "Alex", "Kai_Builds", "LunaCraft"];
  if (fast) roster.slice(0, 3).forEach((n) => join(id, n));
  else {
    roster.slice(0, 3).forEach((n, i) => own.push(window.setTimeout(() => join(id, n), 2500 + i * 3500)));
  }
  const chat = [
    "<Steve> ¿alguien tiene hierro?",
    "<Alex> voy a por la granja de aldeanos",
    "<Kai_Builds> mirad el castillo",
  ];
  own.push(
    window.setInterval(() => {
      const line = chat[Math.floor(Math.random() * chat.length)];
      log(id, line, "INFO", "Async Chat Thread - #0");
    }, 5200),
  );
}

async function stop(id: string) {
  setStatus(id, "stopping");
  log(id, "Stopping the server");
  log(id, "Stopping server");
  log(id, "Saving players");
  for (const name of servers.find((s) => s.config.id === id)?.players ?? []) leave(id, name);
  log(id, "Saving worlds");
  await sleep(900);
  log(id, "ThreadedAnvilChunkStorage: All dimensions are saved");
  timers.get(id)?.forEach((t) => (window.clearInterval(t), window.clearTimeout(t)));
  setStatus(id, "stopped");
  emit("server://event", { serverId: id, event: { type: "exited", code: 0 } });
}

function command(id: string, raw: string) {
  const [verb, ...rest] = raw.split(" ");
  const arg = rest.join(" ");
  const players = servers.find((s) => s.config.id === id)?.players ?? [];
  switch (verb) {
    case "say":
      log(id, `[Server] ${arg}`);
      break;
    case "op":
      log(id, `Made ${arg} a server operator`);
      break;
    case "kick":
      log(id, `Kicked ${arg}: Kicked by an operator`);
      if (players.includes(arg)) leave(id, arg);
      break;
    case "list":
      log(id, `There are ${players.length} of a max of 20 players online: ${players.join(", ")}`);
      break;
    case "time":
      log(id, "Set the time to 1000");
      break;
    default:
      log(id, "Unknown or incomplete command, see below for error", "INFO");
      log(id, `${raw}<--[HERE]`);
  }
}

const VERSIONS = [
  "1.21.4",
  "1.21.3",
  "1.21.1",
  "1.21",
  "1.20.6",
  "1.20.4",
  "1.20.1",
  "1.19.4",
  "1.18.2",
  "1.16.5",
  "1.12.2",
];

const PACKS = [
  [
    "aventura-cosmica",
    "Aventura Cósmica",
    "estelar",
    "Explora planetas, construye cohetes y sobrevive en el espacio con más de 200 mods.",
    2_400_000,
    "1.20.1",
  ],
  [
    "tecnologia-total",
    "Tecnología Total",
    "ingenia",
    "Automatiza todo: fábricas, energía, tuberías y robots. Para quienes disfrutan optimizando.",
    1_350_000,
    "1.20.1",
  ],
  [
    "magia-y-dragones",
    "Magia y Dragones",
    "runa",
    "Hechizos, mazmorras y jefes épicos en un mundo de fantasía renovado.",
    980_000,
    "1.19.2",
  ],
  [
    "supervivencia-extrema",
    "Supervivencia Extrema",
    "frio",
    "Temperatura, sed y criaturas más duras. Cada noche es un reto.",
    640_000,
    "1.21.1",
  ],
  [
    "ciudades-medievales",
    "Ciudades Medievales",
    "castillo",
    "Aldeas más vivas, estructuras nuevas y comercio entre reinos.",
    410_000,
    "1.20.1",
  ],
  [
    "optimizado-plus",
    "Optimizado Plus",
    "rapido",
    "Vanilla de siempre con mods de rendimiento: más FPS y menos lag para el servidor.",
    3_100_000,
    "1.21.4",
  ],
] as const;

async function provision(req: NewServerRequest): Promise<ServerConfig> {
  const step = (payload: unknown) => emit("provision://progress", payload);
  const modpack = req.source.kind === "modrinth" || req.source.kind === "curseForge";
  await step({ step: "fetchingContent" });
  await sleep(500);
  if (modpack) {
    for (let done = 0; done <= 186; done += 31) {
      await step({ step: "downloadingFiles", done, total: 186 });
      await sleep(250);
    }
  }
  await step({ step: "installingJava", major: 21 });
  const total = 48 * 1024 * 1024;
  for (let downloaded = 0; downloaded <= total; downloaded += total / 8) {
    await step({ step: "downloadingJava", downloaded, total });
    await sleep(180);
  }
  await step({ step: "installingLoader" });
  await sleep(800);
  await step({ step: "finalizing" });
  await sleep(400);

  const id =
    req.name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "") || "servidor";
  const config: ServerConfig = {
    id,
    name: req.name,
    mcVersion: "mcVersion" in req.source ? req.source.mcVersion : "1.20.1",
    loader:
      req.source.kind === "fabric"
        ? { kind: "fabric", loaderVersion: "0.16.9" }
        : modpack
          ? { kind: "forge", version: "47.3.0" }
          : { kind: "vanilla" },
    javaMajor: 21,
    memory: req.memory,
    cpu: req.cpu,
    port: req.port,
    eulaAccepted: req.eulaAccepted,
  };
  servers = [...servers, { config, status: "stopped", players: [], dir: `${DATA_DIR}/servers/${id}` }];
  return config;
}

mockIPC(
  async (cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case "app_info":
        return { version: "0.1.0", dataDir: DATA_DIR, curseforgeEnabled: false };
      case "hardware_info":
        return { logicalCpus: 12, totalMemoryMb: 16384, recommendedMaxMb: 12288 };
      case "network_info":
        return { localIp: "192.168.1.42" };
      case "list_java_runtimes":
        return [
          { major: 17, releaseName: "jdk-17.0.13+11", home: "", javaBin: "" },
          { major: 21, releaseName: "jdk-21.0.5+11", home: "", javaBin: "" },
        ];
      case "minecraft_versions":
        await sleep(300);
        return VERSIONS.map((id) => ({ id, type: "release", url: "", sha1: "" }));
      case "list_servers":
        return servers;
      case "start_server":
        start(a.id as string);
        return null;
      case "stop_server":
        await stop(a.id as string);
        return null;
      case "send_command":
        command(a.id as string, a.command as string);
        return null;
      case "create_server":
        return provision(a.request as NewServerRequest);
      case "search_modrinth": {
        await sleep(350);
        const q = String((a.query as { query: string }).query).toLowerCase();
        const hits = PACKS.filter((p) => !q || p[1].toLowerCase().includes(q) || p[3].toLowerCase().includes(q)).map(
          ([id, title, author, description, downloads, version]) => ({
            project_id: id,
            slug: id,
            title,
            author,
            description,
            downloads,
            icon_url: null,
            categories: [],
            versions: [version],
          }),
        );
        return { hits, total_hits: hits.length, offset: 0, limit: 24 };
      }
      case "modrinth_versions":
        await sleep(300);
        return ["2.4.1", "2.4.0", "2.3.2"].map((v, i) => ({
          id: `${a.projectId}-${v}`,
          name: v,
          version_number: v,
          version_type: i ? "release" : "release",
          game_versions: ["1.20.1"],
          loaders: ["fabric"],
          date_published: "2026-09-01",
        }));
      default:
        // open_server_folder, open_data_folder, plugin:opener|open_url…
        console.info("[mock]", cmd, args);
        return null;
    }
  },
  { shouldMockEvents: true },
);

if (scenario === "running") {
  setTimeout(() => start("survival-con-amigos", true), 300);
}
