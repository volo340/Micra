// Backend simulado para `npm run dev` en un navegador normal (sin Tauri).
// Sirve para diseñar y probar la interfaz; nunca se incluye en la app final.
//
// Parámetros de la URL:
//   ?state=empty      sin servidores (pantalla de bienvenida)
//   ?state=running    el primer servidor arranca solo, con jugadores y gráficas
//   ?lang=en          idioma del "sistema": es, en, pt, fr o de
//   ?platform=windows ajustes y rutas de Windows
//   ?update=1         hay una versión nueva de Micra
//   ?malware=1        el análisis de mods encuentra algo
//   ?modfail=1        el asistente no consigue instalar el segundo mod elegido
//   ?crash=1          «Aventura con mods» no arrancó: Micra señala qué mods quitar y cuál falta
//   ?crash=lost       igual, pero no se encuentra dónde descargar el que falta
//   ?crash=memory     «Aventura con mods» se quedó sin memoria
//   ?nocf=1           sin clave de CurseForge: solo se busca en Modrinth
//
// Una clave de CurseForge que empiece por cfc_pat_ se rechaza, como la «Profile API key».
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import { loaderName } from "../lib/format";
import type {
  AppSettings,
  BackupInfo,
  BackupScope,
  ContentHit,
  ContentPage,
  ContentQuery,
  CrashReport,
  ImportAnalysis,
  ImportRequest,
  Lang,
  LiveInfo,
  ModInfo,
  ModRemoteInfo,
  NewServerRequest,
  PlayerAction,
  PlayerLists,
  PropertiesView,
  ScanReport,
  SearchCursor,
  SearchSource,
  SecretKey,
  ServerConfig,
  ServerSettings,
  ServerStatus,
  ServerSummary,
  SettingsView,
  StatPoint,
  StatsRange,
  TaskKind,
} from "../lib/types";

const params = new URLSearchParams(location.search);
const scenario = params.get("state");
const platform = params.get("platform") === "windows" ? "windows" : "linux";
const LANGS: Lang[] = ["es", "en", "pt", "fr", "de"];
const systemLang: Lang = LANGS.includes(params.get("lang") as Lang) ? (params.get("lang") as Lang) : "es";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const DATA_DIR =
  platform === "windows" ? "C:\\Users\\Steve\\AppData\\Roaming\\micra" : "/home/steve/.local/share/micra";
const SEP = platform === "windows" ? "\\" : "/";
const LOCAL_IP = "192.168.1.42";
const PUBLIC_IP = "203.0.113.24";
const serverDir = (id: string) => [DATA_DIR, "servers", id].join(SEP);
const hoursAgo = (h: number) => new Date(Date.now() - h * 3_600_000).toISOString();

/** Generador pseudoaleatorio con semilla, para que las gráficas no cambien al recargar. */
function seeded(seed: number) {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const fakeHash = (text: string) =>
  Array.from({ length: 40 }, (_, i) => ((text.charCodeAt(i % text.length) * (i + 7)) % 16).toString(16)).join("");

const fakeUuid = (name: string) => {
  const h = fakeHash(name);
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-4${h.slice(13, 16)}-a${h.slice(17, 20)}-${h.slice(20, 32)}`;
};

// ---------- Servidores ----------

type Base = Pick<ServerConfig, "id" | "name" | "mcVersion" | "loader" | "javaMajor" | "memory" | "cpu" | "port">;

function makeConfig(base: Base & Partial<ServerConfig>): ServerConfig {
  return {
    eulaAccepted: true,
    launch: { kind: "jar", path: "server.jar" },
    useAikarFlags: true,
    extraJvmArgs: [],
    backups: { enabled: true, intervalHours: 6, keep: 10 },
    schedule: { dailyRestart: null, restartEveryHours: null, idleShutdownMinutes: null },
    network: { upnp: false, tunnel: false },
    security: {
      guard: { enabled: true, maxAttempts: 8, windowSecs: 60, banMinutes: 60, maxPerIp: 4 },
      sandbox: true,
      lowIntegrity: false,
    },
    map: { enabled: false, port: 8100, acceptedDownload: false },
    source: null,
    pendingOwner: null,
    ...base,
  };
}

const idleLive = (): LiveInfo => ({
  tps: null,
  upnp: null,
  tunnel: null,
  restartAt: null,
  packUpdate: null,
  modUpdates: 0,
  sandbox: null,
  mapNeedsAccept: false,
  lastBackupError: null,
  crash: null,
});

const summary = (config: ServerConfig, live: Partial<LiveInfo> = {}): ServerSummary => ({
  config,
  status: "stopped",
  players: [],
  dir: serverDir(config.id),
  live: { ...idleLive(), ...live },
  busy: false,
  startedAt: null,
});

let servers: ServerSummary[] =
  scenario === "empty"
    ? []
    : [
        summary(
          makeConfig({
            id: "survival-con-amigos",
            name: "Survival con amigos",
            mcVersion: "26.3",
            loader: { kind: "vanilla" },
            javaMajor: 25,
            memory: { minMb: 2048, maxMb: 4096 },
            cpu: { cores: null },
            port: 25565,
            schedule: { dailyRestart: "05:00", restartEveryHours: null, idleShutdownMinutes: null },
            network: { upnp: true, tunnel: false },
          }),
        ),
        summary(
          makeConfig({
            id: "aventura-con-mods",
            name: "Aventura con mods",
            mcVersion: "1.20.1",
            loader: { kind: "fabric", loaderVersion: "0.16.9" },
            javaMajor: 17,
            memory: { minMb: 2048, maxMb: 6144 },
            cpu: { cores: [0, 1, 2, 3, 4, 5] },
            port: 25566,
            launch: { kind: "jar", path: "fabric-server-launch.jar" },
            source: { kind: "modrinth", projectId: "aventura-cosmica", versionId: "aventura-cosmica-2.3.2" },
            map: { enabled: true, port: 8100, acceptedDownload: false },
          }),
          {
            packUpdate: {
              current: "2.3.2",
              latest: "2.4.1",
              target: { kind: "modrinth", versionId: "aventura-cosmica-2.4.1" },
            },
            modUpdates: 2,
            mapNeedsAccept: true,
          },
        ),
      ];

/** Lo que Micra lee del error de un servidor de Fabric que no arranca. */
const CRASH: CrashReport = {
  culprits: [
    {
      fileName: "iris-mc1.20.1-1.6.11.jar",
      modId: "iris",
      name: "Iris",
      reason: { kind: "clientOnly" },
      confidence: "certain",
      evidence:
        "java.lang.RuntimeException: Cannot load class net.minecraft.class_310 in environment type SERVER\n" +
        "\tat net.coderbot.iris.Iris.onEarlyInitialize(Iris.java:124)",
    },
    {
      fileName: "create-fabric-0.5.1-f-build.1417+mc1.20.1.jar",
      modId: "create",
      name: "Create",
      reason: { kind: "suspected" },
      confidence: "possible",
      evidence:
        "java.lang.RuntimeException: Cannot load class net.minecraft.class_310 in environment type SERVER\n" +
        "\tat com.simibubi.create.Create.onInitialize(Create.java:153)",
    },
  ],
  missing: [
    {
      id: "cloth-config",
      version: "≥ 11.1.106",
      requiredBy: [{ fileName: "betterhud-fabric-1.2.0.jar", name: "Better HUD" }],
      disabledFile: null,
      found: null,
    },
  ],
  cause: null,
  headline: "java.lang.RuntimeException: Cannot load class net.minecraft.class_310 in environment type SERVER",
  reportFile: "crash-reports/crash-2026-10-24_12.30.11-server.txt",
  duringStart: true,
  clientClasses: true,
  missingChecked: false,
};

/** ?crash=memory: se quedó sin memoria, no es culpa de ningún mod. */
const MEMORY_CRASH: CrashReport = {
  culprits: [],
  missing: [],
  cause: { kind: "outOfMemory" },
  headline: "java.lang.OutOfMemoryError: Java heap space",
  reportFile: null,
  duringStart: false,
  clientClasses: false,
  missingChecked: true,
};

if (params.get("crash")) {
  const crash = params.get("crash") === "memory" ? MEMORY_CRASH : CRASH;
  servers = servers.map((s) =>
    s.config.id === "aventura-con-mods" ? { ...s, status: "crashed", live: { ...s.live, crash } } : s,
  );
  // Como en la app: dónde descargar lo que falta llega un poco después.
  setTimeout(() => {
    const live = servers.find((s) => s.config.id === "aventura-con-mods")?.live;
    if (!live?.crash) return;
    setLive("aventura-con-mods", {
      crash: {
        ...live.crash,
        missing: live.crash.missing.map((m) =>
          m.id === "cloth-config" && params.get("crash") !== "lost"
            ? { ...m, found: { source: "modrinth", projectId: "9s6osm5g", title: "Cloth Config API" } }
            : m,
        ),
        missingChecked: true,
      },
    });
  }, 1500);
}

/** Cambia el aviso de por qué falló el último arranque, si lo hay. */
function updateCrash(id: string, change: (crash: CrashReport) => CrashReport) {
  const crash = find(id).live.crash;
  if (crash) setLive(id, { crash: change(crash) });
}

/** Deja de pedir los mods que faltaban y ya están instalados y activos. */
const forgetInstalled = (id: string) =>
  updateCrash(id, (crash) => ({
    ...crash,
    missing: crash.missing.filter((m) => !modsOf(id).some((x) => x.enabled && x.modId === m.id)),
  }));

const find = (id: string) => {
  const s = servers.find((x) => x.config.id === id);
  if (!s) throw `No existe el servidor ${id}`;
  return s;
};

function update(id: string, change: (s: ServerSummary) => ServerSummary) {
  servers = servers.map((s) => (s.config.id === id ? change(s) : s));
}

function setLive(id: string, patch: Partial<LiveInfo>) {
  update(id, (s) => ({ ...s, live: { ...s.live, ...patch } }));
  emit("server://live", { serverId: id, live: find(id).live });
}

const timers = new Map<string, number[]>();

const clock = () => new Date().toTimeString().slice(0, 8);
const log = (serverId: string, message: string, level = "INFO", thread = "Server thread") =>
  emit("server://event", {
    serverId,
    event: { type: "log", stream: "stdout", line: `[${clock()}] [${thread}/${level}]: ${message}` },
  });

function setStatus(id: string, status: ServerStatus) {
  update(id, (s) => ({
    ...s,
    status,
    startedAt: status === "running" ? new Date().toISOString() : status === "starting" ? s.startedAt : null,
  }));
  emit("server://event", { serverId: id, event: { type: "status", status } });
}

function setPlayers(id: string, change: (p: string[]) => string[]) {
  update(id, (s) => ({ ...s, players: change(s.players) }));
}

function join(id: string, name: string) {
  setPlayers(id, (p) => [...p, name]);
  log(
    id,
    `${name}[/192.168.1.${20 + name.length}:5${name.length}812] logged in with entity id 1${name.length}2 at (12.5, 64.0, -30.5)`,
  );
  log(id, `${name} joined the game`);
  emit("server://event", { serverId: id, event: { type: "playerJoined", name } });
  known(id, name);
}

function leave(id: string, name: string, reason = "Disconnected") {
  setPlayers(id, (p) => p.filter((x) => x !== name));
  log(id, `${name} lost connection: ${reason}`);
  log(id, `${name} left the game`);
  emit("server://event", { serverId: id, event: { type: "playerLeft", name } });
}

function reject(id: string, name: string) {
  log(id, `Disconnecting ${name} (/198.51.100.23:51234): You are not white-listed on this server!`);
  emit("server://event", { serverId: id, event: { type: "joinRejected", name, reason: "notWhitelisted" } });
  known(id, name);
}

/** Próxima vez que el reloj marque "HH:MM". */
function nextAt(time: string): string {
  const [h, m] = time.split(":").map(Number);
  const at = new Date();
  at.setHours(h, m, 0, 0);
  if (at.getTime() <= Date.now()) at.setDate(at.getDate() + 1);
  return at.toISOString();
}

const STARTUP = [
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
  let tps = 20;
  const point = () => {
    const s = servers.find((x) => x.config.id === id);
    const players = s?.players.length ?? 0;
    const max = s?.config.memory.maxMb ?? 4096;
    cpu = Math.max(3, Math.min(92, cpu + (Math.random() - 0.5) * 9 + (players * 6 - cpu) * 0.08));
    mem = Math.max(1200, Math.min(max * 0.95, mem + (Math.random() - 0.4) * 45));
    tps = Math.max(14, Math.min(20, tps + (Math.random() - 0.35) * 0.4));
    emit("server://stats", {
      serverId: id,
      sample: { cpuPercent: cpu, memoryMb: mem },
      playerCount: players,
      tps: Math.round(tps * 10) / 10,
    });
  };
  for (let i = 0; i < history; i++) point();
  return window.setInterval(point, 1000);
}

async function start(id: string, fast = false) {
  const { config } = find(id);
  setStatus(id, "starting");
  const own: number[] = [];
  timers.set(id, own);
  setLive(id, {
    crash: null,
    sandbox: config.security.sandbox
      ? platform === "windows"
        ? { kind: "jobObject", lowIntegrity: config.security.lowIntegrity }
        : { kind: "landlock", abi: 6, seccomp: true }
      : { kind: "disabled" },
    upnp: config.network.upnp ? { kind: "opening" } : null,
    tunnel: config.network.tunnel ? { kind: "starting" } : null,
    restartAt: config.schedule.dailyRestart ? nextAt(config.schedule.dailyRestart) : null,
  });
  log(id, `Starting minecraft server version ${config.mcVersion}`);
  for (const line of STARTUP) {
    log(id, line, "INFO", line.startsWith("Preparing") ? "Worker-Main-2" : "Server thread");
    await sleep(fast ? 5 : 120);
  }
  log(id, "Can't keep up! Is the server overloaded? Running 2044ms or 40 ticks behind", "WARN");
  log(id, 'Done (3.214s)! For help, type "help"');
  setStatus(id, "running");
  // Lo pendiente para el mundo se aplica al arrancar.
  const { rules } = propertiesOf(id);
  const values = properties.get(id)?.values ?? {};
  for (const key of worldPending.get(id) ?? []) {
    if (key in rules) log(id, `Gamerule ${rules[key]} is now set to: ${values[key] ?? "true"}`);
  }
  worldPending.delete(id);
  own.push(startStats(id, fast ? 48 : 0));
  const live = find(id).config.network;
  own.push(
    window.setTimeout(
      () =>
        setLive(id, {
          tps: 19.9,
          upnp: live.upnp
            ? {
                kind: "open",
                mapping: {
                  port: config.port,
                  localIp: LOCAL_IP,
                  externalIp: PUBLIC_IP,
                  wan: "public",
                  permanent: false,
                },
              }
            : null,
          tunnel: live.tunnel ? { kind: "ready", address: "verde-castillo.joinmc.link" } : null,
        }),
      fast ? 10 : 1500,
    ),
  );

  const roster = ["Steve", "Alex", "Kai_Builds"];
  if (fast) roster.forEach((n) => join(id, n));
  else roster.forEach((n, i) => own.push(window.setTimeout(() => join(id, n), 2500 + i * 3500)));
  own.push(window.setTimeout(() => reject(id, "LunaCraft"), fast ? 600 : 12000));
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
  for (const name of find(id).players) leave(id, name);
  log(id, "Saving worlds");
  await sleep(900);
  log(id, "ThreadedAnvilChunkStorage: All dimensions are saved");
  timers.get(id)?.forEach((t) => (window.clearInterval(t), window.clearTimeout(t)));
  setStatus(id, "stopped");
  setLive(id, { tps: null, upnp: null, tunnel: null, restartAt: null, sandbox: null });
  emit("server://event", { serverId: id, event: { type: "exited", code: 0 } });
}

function command(id: string, raw: string) {
  const [verb, ...rest] = raw.split(" ");
  const arg = rest.join(" ");
  const players = find(id).players;
  switch (verb) {
    case "say":
      log(id, `[Server] ${arg}`);
      break;
    case "op":
      log(id, `Made ${arg} a server operator`);
      break;
    case "kick":
      log(id, `Kicked ${rest[0]}: ${rest.slice(1).join(" ") || "Kicked by an operator"}`);
      if (players.includes(rest[0])) leave(id, rest[0], "Kicked by an operator");
      break;
    case "list":
      log(id, `There are ${players.length} of a max of 20 players online: ${players.join(", ")}`);
      break;
    case "time":
      log(id, "Set the time to 1000");
      break;
    case "whitelist":
      log(id, rest[0] === "on" ? "Whitelist is now turned on" : `Added ${rest[1]} to the whitelist`);
      break;
    default:
      log(id, "Unknown or incomplete command, see below for error", "INFO");
      log(id, `${raw}<--[HERE]`);
  }
}

/** Progreso de una tarea larga, como el que emite el backend. */
async function task(serverId: string | null, kind: TaskKind, total: number, ms: number) {
  const steps = 12;
  for (let i = 0; i <= steps; i++) {
    emit("task://progress", { serverId, task: kind, done: Math.round((total * i) / steps), total });
    await sleep(ms / steps);
  }
}

// ---------- Ajustes de servidor ----------

function applySettings(id: string, s: ServerSettings): { config: ServerConfig; restartNeeded: boolean } {
  const before = find(id);
  const config: ServerConfig = {
    ...before.config,
    memory: s.memory,
    cpu: s.cpu,
    port: s.port,
    useAikarFlags: s.useAikarFlags,
    backups: s.backups,
    schedule: s.schedule,
    network: s.network,
    security: s.security,
    map: { ...before.config.map, port: s.mapPort },
  };
  const running = before.status === "running" || before.status === "starting";
  const old = before.config;
  const restartNeeded =
    running &&
    JSON.stringify([old.memory, old.cpu, old.port, old.useAikarFlags, old.security]) !==
      JSON.stringify([config.memory, config.cpu, config.port, config.useAikarFlags, config.security]);
  update(id, (x) => ({ ...x, config }));
  if (running) {
    setLive(id, { restartAt: config.schedule.dailyRestart ? nextAt(config.schedule.dailyRestart) : null });
  }
  return { config, restartNeeded };
}

// ---------- Jugadores ----------

const ban = (name: string, reason: string, expires = "forever") => ({
  uuid: fakeUuid(name),
  name,
  created: hoursAgo(30),
  source: "Micra",
  expires,
  reason,
});

const players = new Map<string, PlayerLists>([
  [
    "survival-con-amigos",
    {
      whitelist: ["volo", "Steve", "Alex", "Kai_Builds"].map((name) => ({ uuid: fakeUuid(name), name })),
      ops: [{ uuid: fakeUuid("volo"), name: "volo", level: 4, bypassesPlayerLimit: false }],
      bannedPlayers: [ban("Griefer123", "Romper casas ajenas")],
      bannedIps: [
        {
          ip: "198.51.100.77",
          created: hoursAgo(5),
          source: "Micra",
          expires: hoursAgo(-1),
          reason: "Demasiados intentos de entrar",
        },
      ],
      known: ["volo", "Steve", "Alex", "Kai_Builds", "Griefer123"].map((name) => ({ name, uuid: fakeUuid(name) })),
    },
  ],
  [
    "aventura-con-mods",
    {
      whitelist: ["volo", "Alex"].map((name) => ({ uuid: fakeUuid(name), name })),
      ops: [{ uuid: fakeUuid("volo"), name: "volo", level: 4, bypassesPlayerLimit: false }],
      bannedPlayers: [],
      bannedIps: [],
      known: ["volo", "Alex"].map((name) => ({ name, uuid: fakeUuid(name) })),
    },
  ],
]);

function listsOf(id: string): PlayerLists {
  if (!players.has(id)) players.set(id, { whitelist: [], ops: [], bannedPlayers: [], bannedIps: [], known: [] });
  return players.get(id)!;
}

function known(id: string, name: string) {
  const l = listsOf(id);
  if (!l.known.some((k) => k.name === name)) l.known.push({ name, uuid: fakeUuid(name) });
}

function playerAction(id: string, action: PlayerAction, target: string) {
  const l = listsOf(id);
  const running = find(id).status === "running";
  const online = find(id).players.includes(target);
  const say = (msg: string) => running && log(id, msg);
  switch (action.kind) {
    case "whitelistAdd":
      if (!l.whitelist.some((w) => w.name === target)) l.whitelist.push({ uuid: fakeUuid(target), name: target });
      say(`Added ${target} to the whitelist`);
      break;
    case "whitelistRemove":
      l.whitelist = l.whitelist.filter((w) => w.name !== target);
      say(`Removed ${target} from the whitelist`);
      break;
    case "op":
      if (!l.ops.some((o) => o.name === target))
        l.ops.push({ uuid: fakeUuid(target), name: target, level: 4, bypassesPlayerLimit: false });
      say(`Made ${target} a server operator`);
      break;
    case "deop":
      l.ops = l.ops.filter((o) => o.name !== target);
      say(`Made ${target} no longer a server operator`);
      break;
    case "ban":
      l.bannedPlayers = [...l.bannedPlayers.filter((b) => b.name !== target), ban(target, action.reason)];
      say(`Banned ${target}: ${action.reason || "Banned by an operator."}`);
      if (online) leave(id, target, "You are banned from this server.");
      break;
    case "pardon":
      l.bannedPlayers = l.bannedPlayers.filter((b) => b.name !== target);
      say(`Unbanned ${target}`);
      break;
    case "banIp":
      l.bannedIps.push({
        ip: target,
        created: new Date().toISOString(),
        source: "Micra",
        expires: "forever",
        reason: action.reason,
      });
      break;
    case "pardonIp":
      l.bannedIps = l.bannedIps.filter((b) => b.ip !== target);
      break;
    case "kick":
      if (!running) throw "El servidor está apagado";
      log(id, `Kicked ${target}: ${action.reason || "Kicked by an operator"}`);
      if (online) leave(id, target, "Kicked by an operator");
      break;
  }
}

// ---------- Copias ----------

const backupName = (date: Date, kind: string, scope: BackupScope) =>
  `${date.toISOString().slice(0, 19).replace("T", "_").replace(/:/g, "-")}-${kind}-${scope}.zip`;

function backup(h: number, kind: BackupInfo["kind"], scope: BackupScope, size: number, mcVersion: string): BackupInfo {
  const date = new Date(Date.now() - h * 3_600_000);
  return { fileName: backupName(date, kind, scope), kind, scope, createdAt: date.toISOString(), size, mcVersion };
}

const backups = new Map<string, BackupInfo[]>([
  [
    "survival-con-amigos",
    [
      backup(1.5, "auto", "world", 412_000_000, "26.3"),
      backup(7.5, "auto", "world", 409_000_000, "26.3"),
      backup(13.5, "auto", "world", 401_000_000, "26.3"),
      backup(26, "manual", "full", 642_000_000, "26.3"),
      backup(50, "preRestore", "world", 388_000_000, "26.3"),
    ],
  ],
  [
    "aventura-con-mods",
    [backup(3, "auto", "world", 1_210_000_000, "1.20.1"), backup(72, "preUpdate", "full", 2_640_000_000, "1.20.1")],
  ],
]);

const backupsOf = (id: string) => backups.get(id) ?? [];

// ---------- Mods ----------

function mod(
  fileName: string,
  name: string,
  version: string,
  modId: string,
  fromPack = true,
  enabled = true,
  clientOnly = false,
): ModInfo {
  return {
    fileName: enabled ? fileName : `${fileName}.disabled`,
    enabled,
    size: 200_000 + (fakeHash(fileName).charCodeAt(3) % 40) * 90_000,
    sha1: fakeHash(fileName),
    modId,
    name,
    version,
    fromPack,
    clientOnly,
  };
}

const mods = new Map<string, ModInfo[]>([
  [
    "aventura-con-mods",
    [
      mod("fabric-api-0.92.2+1.20.1.jar", "Fabric API", "0.92.2+1.20.1", "fabric-api"),
      mod("lithium-fabric-mc1.20.1-0.11.2.jar", "Lithium", "0.11.2", "lithium"),
      mod("ferritecore-6.0.1-fabric.jar", "FerriteCore", "6.0.1", "ferritecore"),
      mod("create-fabric-0.5.1-f-build.1417+mc1.20.1.jar", "Create", "0.5.1-f", "create"),
      mod("waystones-fabric-1.20-14.1.3.jar", "Waystones", "14.1.3", "waystones"),
      mod("balm-fabric-1.20.1-7.3.9.jar", "Balm", "7.3.9", "balm"),
      mod("jei-1.20.1-fabric-15.3.0.4.jar", "Just Enough Items", "15.3.0.4", "jei", true, false),
      mod("Chunky-1.3.146.jar", "Chunky", "1.3.146", "chunky", false),
      ...(params.get("crash")
        ? [
            mod("iris-mc1.20.1-1.6.11.jar", "Iris", "1.6.11", "iris", true, true, true),
            mod("sodium-fabric-mc1.20.1-0.5.3.jar", "Sodium", "0.5.3", "sodium"),
            mod("betterhud-fabric-1.2.0.jar", "Better HUD", "1.2.0", "betterhud", false),
          ]
        : []),
    ],
  ],
]);

const modsOf = (id: string) => mods.get(id) ?? [];

/** Mods que Modrinth dice que solo funcionan en el juego (cliente). */
const CLIENT_ONLY = new Set(["sodium", "iris"]);
/** Mods instalados desde CurseForge: SHA-1 -> id del proyecto. */
const fromCurseForge = new Map<string, string>([[fakeHash("Chunky-1.3.146.jar"), "396246"]]);

/** Mods con versión nueva en Modrinth: id del mod -> [versión, archivo]. */
const UPDATES: Record<string, [string, string]> = {
  lithium: ["0.11.3", "lithium-fabric-mc1.20.1-0.11.3.jar"],
  chunky: ["1.3.158", "Chunky-1.3.158.jar"],
};

function remoteInfo(id: string): ModRemoteInfo[] {
  return modsOf(id)
    .filter((m) => m.modId)
    .map((m) => {
      const up = UPDATES[m.modId!];
      const curseforge = fromCurseForge.get(m.sha1);
      return {
        sha1: m.sha1,
        source: curseforge ? "curseforge" : "modrinth",
        projectId: curseforge ?? m.modId!,
        title: m.name,
        iconUrl: null,
        versionNumber: m.version ?? "?",
        clientOnly: CLIENT_ONLY.has(m.modId!),
        update:
          up && up[0] !== m.version
            ? { versionId: `${m.modId}-${up[0]}`, versionNumber: up[0], fileName: up[1] }
            : null,
      };
    });
}

const ALL_LOADERS = ["fabric", "forge", "neoforge"];

/** id, nombre, autor, descripción, descargas y loaders (nombres de Modrinth). */
const MOD_CATALOG: [string, string, string, string, number, string[]][] = [
  [
    "lithium",
    "Lithium",
    "jellysquid3",
    "No-compromises game logic optimization mod.",
    31_200_000,
    ["fabric", "neoforge"],
  ],
  ["ferritecore", "FerriteCore", "malte0811", "Memory usage optimizations.", 25_400_000, ALL_LOADERS],
  [
    "create",
    "Create",
    "simibubi",
    "Building tools and aesthetic technology: gears, belts and contraptions.",
    21_700_000,
    ["forge", "neoforge"],
  ],
  ["jei", "Just Enough Items", "mezz", "View items and recipes.", 19_300_000, ALL_LOADERS],
  ["spark", "spark", "lucko", "A performance profiler: find out what is causing lag.", 9_800_000, ALL_LOADERS],
  [
    "farmers-delight",
    "Farmer's Delight",
    "vectorwing",
    "Farming and cooking, with new crops and meals.",
    8_900_000,
    ["forge", "neoforge"],
  ],
  ["chunky", "Chunky", "pop4959", "Pre-generates chunks, quickly and efficiently.", 7_400_000, ALL_LOADERS],
  [
    "mekanism",
    "Mekanism",
    "bradyaidanc",
    "Machines, energy and ore processing for technology fans.",
    6_200_000,
    ["neoforge"],
  ],
  ["waystones", "Waystones", "BlayTheNinth", "Teleport back to activated waystones.", 5_600_000, ALL_LOADERS],
  ["krypton", "Krypton", "astei", "A mod to optimize the Minecraft networking stack.", 5_100_000, ["fabric"]],
  [
    "modernfix",
    "ModernFix",
    "embeddedt",
    "Fixes and speedups for faster launch and less memory.",
    4_800_000,
    ALL_LOADERS,
  ],
  [
    "c2me-fabric",
    "Concurrent Chunk Management Engine",
    "ishland",
    "Improves chunk performance.",
    4_300_000,
    ["fabric"],
  ],
  [
    "sophisticated-backpacks",
    "Sophisticated Backpacks",
    "P3pp3rF1y",
    "Backpacks with upgrades: sorting, pickup, crafting.",
    3_900_000,
    ["forge", "neoforge"],
  ],
  ["bluemap", "BlueMap", "Blue", "A 3D map of your world in the browser.", 1_900_000, ALL_LOADERS],
  ["styled-chat", "Styled Chat", "Patbox", "Colours and formatting for the server chat.", 1_200_000, ["fabric"]],
  ["ledger", "Ledger", "PotatoPresident", "Server-side logging: who broke or placed each block.", 800_000, ["fabric"]],
];

/** Los de CurseForge: id, nombre, autor, descripción, descargas y loaders. */
const CF_MOD_CATALOG: [number, string, string, string, number, string[]][] = [
  [238222, "Just Enough Items (JEI)", "mezz", "View items and recipes.", 310_000_000, ALL_LOADERS],
  [
    254284,
    "Clumps",
    "Jaredlll08",
    "Groups XP orbs together into a single entity to reduce lag.",
    180_000_000,
    ALL_LOADERS,
  ],
  [
    243121,
    "Corail Tombstone",
    "Corail31",
    "Keeps your items in a grave when you die, with a few magic tricks.",
    95_000_000,
    ["forge", "neoforge"],
  ],
  [
    228756,
    "Torchmaster",
    "xalcon",
    "Mega torches that stop monsters from spawning around them.",
    88_000_000,
    ["forge", "neoforge"],
  ],
  [
    223852,
    "Storage Drawers",
    "Texelsaur",
    "Drawers that hold large amounts of a single item.",
    76_000_000,
    ALL_LOADERS,
  ],
  [314906, "FTB Chunks", "FTB", "Claim and force-load chunks, with a minimap.", 52_000_000, ALL_LOADERS],
  [396246, "Chunky", "pop4959", "Pre-generates chunks, quickly and efficiently.", 7_000_000, ALL_LOADERS],
  [404465, "Iron Chests", "ProgWML6", "Bigger chests made of iron, gold and diamond.", 48_000_000, ALL_LOADERS],
];

/** Loaders de Modrinth cuyos mods carga un servidor. */
const modrinthLoadersOf = (loader: ServerConfig["loader"]) =>
  loader.kind === "neoForge" ? ["neoforge"] : loader.kind === "quilt" ? ["quilt", "fabric"] : [loader.kind];

/** Como el núcleo: minúsculas, sin lo que va entre paréntesis y solo letras y números. */
function dedupeKey(title: string) {
  let depth = 0;
  let out = "";
  for (const c of title) {
    if ("([{".includes(c)) depth++;
    else if (")]}".includes(c)) depth = Math.max(0, depth - 1);
    else if (depth === 0 && /[\p{L}\p{N}]/u.test(c)) out += c.toLowerCase();
  }
  return out || title.trim().toLowerCase();
}

const curseforgeOn = () => secrets.curseforgeApiKey || !params.get("nocf");

const hit = (
  source: SearchSource,
  id: string,
  title: string,
  author: string,
  description: string,
  downloads: number,
  gameVersion: string | null,
): ContentHit => ({
  source,
  id,
  title,
  author,
  description,
  iconUrl: null,
  downloads,
  gameVersion,
  dedupeKey: dedupeKey(title),
});

function sourceHits(source: SearchSource, q: ContentQuery): ContentHit[] {
  const text = q.query.toLowerCase();
  const matches = (title: string, description: string) =>
    !text || title.toLowerCase().includes(text) || description.toLowerCase().includes(text);
  const fits = (own: readonly string[]) => !q.loaders.length || own.some((l) => q.loaders.includes(l));
  if (q.kind === "mod") {
    return source === "modrinth"
      ? MOD_CATALOG.filter(([, title, , desc, , own]) => fits(own) && matches(title, desc)).map(
          ([pid, title, author, desc, downloads]) => hit(source, pid, title, author, desc, downloads, q.gameVersion),
        )
      : CF_MOD_CATALOG.filter(([, title, , desc, , own]) => fits(own) && matches(title, desc)).map(
          ([cid, title, author, desc, downloads]) =>
            hit(source, String(cid), title, author, desc, downloads, q.gameVersion),
        );
  }
  return source === "modrinth"
    ? PACKS.filter((p) => fits(packLoaders(p)) && matches(p[1], p[3])).map((p) =>
        hit(source, p[0], p[1], p[2], p[3], p[4], p[5]),
      )
    : CF_PACKS.map((p, i) => [p, i] as const)
        .filter(([p]) => fits([p[6]]) && matches(p[1], p[3]))
        .map(([p, i]) => hit(source, String(CF_PACK_IDS + i), p[1], p[2], p[3], p[4], p[5]));
}

/** Como el buscador de Micra: una página de cada fuente, mezcladas y sin repetir proyectos. */
function contentPage(q: ContentQuery): ContentPage {
  const size = q.kind === "mod" ? 6 : 12;
  const cursor: SearchCursor = q.cursor ?? { modrinth: 0, curseforge: 0 };
  const enabled = curseforgeOn();
  const page = (source: SearchSource, offset: number | null) => {
    if (offset === null) return { hits: [] as ContentHit[], next: null };
    const all = sourceHits(source, q);
    const hits = all.slice(offset, offset + size);
    const seen = offset + hits.length;
    return { hits, next: hits.length && seen < all.length ? seen : null };
  };
  const m = page("modrinth", cursor.modrinth);
  const c = page("curseforge", enabled ? cursor.curseforge : null);
  const onModrinth = new Set(m.hits.map((h) => h.dedupeKey));
  const rest = c.hits.filter((h) => !onModrinth.has(h.dedupeKey));
  const hits: ContentHit[] = [];
  for (let i = 0; i < Math.max(m.hits.length, rest.length); i++) {
    if (m.hits[i]) hits.push(m.hits[i]);
    if (rest[i]) hits.push(rest[i]);
  }
  const more = m.next !== null || c.next !== null;
  return {
    hits,
    next: more ? { modrinth: m.next, curseforge: c.next } : null,
    failed: [],
    curseforgeEnabled: enabled,
  };
}

// ---------- Propiedades ----------

const rcon = "x7Kp2QmZ9vLc4RtB8nWf3HsJ6yDg1AeU";

const properties = new Map<string, Pick<PropertiesView, "exists" | "values">>([
  [
    "survival-con-amigos",
    {
      exists: true,
      values: {
        motd: "Survival con amigos",
        "max-players": "20",
        difficulty: "normal",
        gamemode: "survival",
        hardcore: "false",
        pvp: "true",
        "level-name": "world",
        "level-seed": "",
        "level-type": "minecraft:normal",
        "view-distance": "10",
        "simulation-distance": "10",
        "spawn-protection": "16",
        "online-mode": "true",
        "white-list": "true",
        "enforce-whitelist": "true",
        "enable-rcon": "true",
        "rcon.port": "25575",
        "rcon.password": rcon,
        "server-port": "25565",
      },
    },
  ],
  [
    "aventura-con-mods",
    {
      exists: false,
      values: {
        "online-mode": "true",
        "white-list": "true",
        "enforce-whitelist": "true",
        "enable-rcon": "true",
        "rcon.port": "25576",
        "rcon.password": rcon.split("").reverse().join(""),
        "server-port": "25566",
      },
    },
  ],
]);

/** Propiedades que Minecraft 1.21.9 convirtió en reglas de juego (como el núcleo). */
const MOVED: [string, string, string][] = [
  ["pvp", "pvp", "pvp"],
  ["enable-command-block", "commandBlocksEnabled", "command_blocks_work"],
  ["allow-nether", "allowEnteringNetherUsingPortals", "allow_entering_nether_using_portals"],
  ["spawn-monsters", "spawnMonsters", "spawn_monsters"],
];

function versionAtLeast(version: string, min: string): boolean {
  const a = version.split(".").map(Number);
  const b = min.split(".").map(Number);
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const d = (a[i] ?? 0) - (b[i] ?? 0);
    if (d !== 0) return d > 0;
  }
  return true;
}

/** Cambios para el mundo que esperan al próximo arranque, por servidor. */
const worldPending = new Map<string, Set<string>>();

function propertiesOf(id: string): PropertiesView {
  const file = properties.get(id) ?? { exists: false, values: {} };
  const mc = servers.find((s) => s.config.id === id)?.config.mcVersion ?? "1.21.4";
  const rules: Record<string, string> = {};
  for (const [key, camel, snake] of MOVED) {
    if (versionAtLeast(mc, "1.21.11")) rules[key] = snake;
    else if (versionAtLeast(mc, "1.21.9")) rules[key] = camel;
  }
  const values = { ...file.values };
  for (const key of Object.keys(rules)) values[key] ??= "true";
  return {
    ...file,
    values,
    rules,
    removed: versionAtLeast(mc, "1.21.2") ? ["spawn-animals", "spawn-npcs"] : [],
    pending: [...(worldPending.get(id) ?? [])],
  };
}

// ---------- Estadísticas ----------

const SPANS: Record<StatsRange, number> = { hour: 3600, day: 86400, week: 7 * 86400, month: 30 * 86400 };
const withHistory = new Set(["survival-con-amigos", "aventura-con-mods"]);

/** Un servidor que se enciende por las tardes, con algo de lag cuando hay mucha gente. */
function history(id: string, range: StatsRange): StatPoint[] {
  if (!withHistory.has(id)) return [];
  const now = Math.floor(Date.now() / 1000);
  const span = SPANS[range];
  const step = Math.max(60, Math.floor(span / 240));
  const maxMb = find(id).config.memory.maxMb;
  const rand = seeded(id.length * 7919 + span);
  const points: StatPoint[] = [];
  for (let ts = now - span; ts <= now; ts += step) {
    const d = new Date(ts * 1000);
    const hour = d.getHours() + d.getMinutes() / 60;
    const on = range === "hour" ? ts > now - 50 * 60 : (hour >= 16 && hour < 23.5) || (d.getDay() === 6 && hour >= 11);
    if (!on) continue;
    const evening = range === "hour" ? 0.75 : Math.max(0, Math.sin(((hour - 15.5) / 8) * Math.PI));
    const count = Math.max(0, Math.round(evening * 5 + (rand() - 0.5) * 2));
    points.push({
      ts,
      cpu: Math.min(95, 8 + count * 9 + rand() * 10),
      ramMb: Math.min(maxMb * 0.95, 1400 + count * 380 + rand() * 300),
      players: count,
      tps: count > 3 && rand() > 0.8 ? 15 + rand() * 3 : 19.4 + rand() * 0.6,
    });
  }
  return points;
}

// ---------- Ajustes de la app ----------

let settings: AppSettings = {
  language: null,
  notifications: {
    desktop: true,
    discord: false,
    playerJoin: true,
    playerLeave: false,
    serverReady: false,
    serverCrash: true,
    security: true,
    backupFailed: true,
  },
  updates: { autoCheck: true },
};

const secrets: Record<SecretKey, boolean> = { discordWebhook: false, curseforgeApiKey: false, playitAgent: false };

const view = (): SettingsView => ({
  settings,
  language: settings.language ?? systemLang,
  secrets: { ...secrets },
  secretBackend: "keyring",
});

let claimPolls = 0;
let wizardInstalls = 0;

// ---------- Crear e importar ----------

const VERSIONS = [
  "26.3",
  "26.1.2",
  "26.1",
  "1.21.11",
  "1.21.10",
  "1.21.9",
  "1.21.8",
  "1.21.4",
  "1.21.1",
  "1.20.6",
  "1.20.4",
  "1.20.1",
  "1.19.2",
  "1.18.2",
  "1.16.5",
  "1.12.2",
];

/** Versiones de Minecraft con cada loader: NeoForge empieza en 1.20.2 y Fabric en 1.14. */
const LOADER_VERSIONS: Record<string, string[]> = {
  fabric: VERSIONS.filter((v) => v !== "1.12.2"),
  forge: VERSIONS,
  neoForge: VERSIONS.slice(0, VERSIONS.indexOf("1.20.4") + 1),
};

/** Java que pide cada versión de Minecraft. */
const javaFor = (mc: string) =>
  /^2\d\./.test(mc) ? 25 : versionAtLeast(mc, "1.20.5") ? 21 : versionAtLeast(mc, "1.18") ? 17 : 8;

/** id, nombre, autor, descripción, descargas, Minecraft y loader. */
type Pack = readonly [string, string, string, string, number, string, string];

const PACKS: Pack[] = [
  [
    "aventura-cosmica",
    "Aventura Cósmica",
    "estelar",
    "Explora planetas, construye cohetes y sobrevive en el espacio con más de 200 mods.",
    2_400_000,
    "1.20.1",
    "forge",
  ],
  [
    "tecnologia-total",
    "Tecnología Total",
    "ingenia",
    "Automatiza todo: fábricas, energía, tuberías y robots. Para quienes disfrutan optimizando.",
    1_350_000,
    "1.21.1",
    "neoforge",
  ],
  [
    "magia-y-dragones",
    "Magia y Dragones",
    "runa",
    "Hechizos, mazmorras y jefes épicos en un mundo de fantasía renovado.",
    980_000,
    "1.19.2",
    "forge",
  ],
  [
    "supervivencia-extrema",
    "Supervivencia Extrema",
    "frio",
    "Temperatura, sed y criaturas más duras. Cada noche es un reto.",
    640_000,
    "1.21.1",
    "fabric",
  ],
  [
    "ciudades-medievales",
    "Ciudades Medievales",
    "castillo",
    "Aldeas más vivas, estructuras nuevas y comercio entre reinos.",
    410_000,
    "1.20.1",
    "fabric",
  ],
  [
    "optimizado-plus",
    "Optimizado Plus",
    "rapido",
    "Vanilla de siempre con mods de rendimiento: más FPS y menos lag para el servidor.",
    3_100_000,
    "1.21.4",
    "fabric",
  ],
];

/** Los de CurseForge: su id es CF_PACK_IDS más su posición. */
const CF_PACK_IDS = 2000;
const CF_PACKS: Pack[] = [
  [
    "reino-de-piedra",
    "Reino de Piedra",
    "cantero",
    "Levanta tu reino: castillos, ejércitos y comercio con otros jugadores.",
    5_800_000,
    "1.20.1",
    "forge",
  ],
  [
    "ingenieria-celestial",
    "Ingeniería Celestial",
    "orbita",
    "Del taller a las estrellas: máquinas, cohetes y estaciones espaciales.",
    2_900_000,
    "1.21.1",
    "neoforge",
  ],
  [
    "islas-flotantes",
    "Islas Flotantes",
    "nube",
    "Empieza en una isla en el cielo y consigue todo desde cero.",
    1_700_000,
    "1.21.1",
    "fabric",
  ],
];

/** Packs con versiones para varios loaders, como algunos de Modrinth: [versión, Minecraft, loader]. */
const PACK_VERSIONS: Record<string, [string, string, string][]> = {
  "supervivencia-extrema": [
    ["1.12.7", "1.21.1", "fabric"],
    ["1.12.6", "1.21.1", "fabric"],
    ["1.9.3", "1.19.2", "forge"],
    ["1.12.5", "1.20.1", "fabric"],
    ["1.9.2", "1.19.2", "forge"],
  ],
};

const packVersions = (p: Pack): [string, string, string][] =>
  PACK_VERSIONS[p[0]] ?? ["2.4.1", "2.4.0", "2.3.2"].map((v) => [v, p[5], p[6]]);
const packLoaders = (p: Pack) => [...new Set(packVersions(p).map(([, , l]) => l))];

const slug = (name: string) =>
  name
    .toLowerCase()
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "") || "servidor";

function uniqueId(name: string) {
  const base = slug(name);
  let id = base;
  for (let i = 2; servers.some((s) => s.config.id === id); i++) id = `${base}-${i}`;
  return id;
}

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
  await step({ step: "installingJava", major: javaFor("mcVersion" in req.source ? req.source.mcVersion : "1.20.1") });
  const total = 48 * 1024 * 1024;
  for (let downloaded = 0; downloaded <= total; downloaded += total / 8) {
    await step({ step: "downloadingJava", downloaded, total });
    await sleep(180);
  }
  await step({ step: "installingLoader" });
  await sleep(800);
  await step({ step: "finalizing" });
  await sleep(400);

  const id = uniqueId(req.name);
  const source = req.source;
  const pack =
    source.kind === "modrinth"
      ? PACKS.find((p) => source.versionId.startsWith(`${p[0]}-`))
      : source.kind === "curseForge"
        ? CF_PACKS[source.modId - CF_PACK_IDS]
        : undefined;
  // La versión elegida del pack dice su Minecraft y su loader.
  const chosen =
    pack && source.kind === "modrinth"
      ? packVersions(pack).find(([v]) => source.versionId === `${pack[0]}-${v}`)
      : undefined;
  const mcVersion = "mcVersion" in source ? source.mcVersion : (chosen?.[1] ?? pack?.[5] ?? "1.20.1");
  const packLoader = chosen?.[2] ?? pack?.[6];
  const kind = packLoader ? (packLoader === "neoforge" ? "neoForge" : packLoader) : source.kind;
  const loader: ServerConfig["loader"] =
    kind === "fabric"
      ? { kind: "fabric", loaderVersion: "0.17.2" }
      : kind === "forge"
        ? { kind: "forge", version: "47.4.0" }
        : kind === "neoForge"
          ? { kind: "neoForge", version: "21.1.209" }
          : { kind: "vanilla" };
  const config = makeConfig({
    id,
    name: req.name,
    mcVersion,
    loader,
    javaMajor: javaFor(mcVersion),
    memory: req.memory,
    cpu: req.cpu,
    port: req.port,
    eulaAccepted: req.eulaAccepted,
    source:
      source.kind === "modrinth"
        ? { kind: "modrinth", projectId: pack?.[0] ?? "aventura-cosmica", versionId: source.versionId }
        : source.kind === "curseForge"
          ? { kind: "curseForge", modId: source.modId, fileId: source.fileId }
          : null,
    pendingOwner: req.owner,
  });
  servers = [...servers, summary(config)];
  if (req.owner) {
    players.set(id, {
      whitelist: [{ uuid: fakeUuid(req.owner), name: req.owner }],
      ops: [{ uuid: fakeUuid(req.owner), name: req.owner, level: 4, bypassesPlayerLimit: false }],
      bannedPlayers: [],
      bannedIps: [],
      known: [],
    });
  }
  return config;
}

const IMPORT_PATH =
  platform === "windows" ? "C:\\Users\\Steve\\Desktop\\servidor-viejo" : "/home/steve/Descargas/servidor-viejo";

const analysis = (path: string): ImportAnalysis => ({
  path,
  suggestedName: path.split(/[\\/]/).pop() ?? "servidor",
  mcVersion: "1.20.4",
  loader: { kind: "fabric", loaderVersion: "0.15.11" },
  launch: { kind: "jar", path: "fabric-server-launch.jar" },
  hasInstaller: false,
  size: 1_930_000_000,
  hasWorld: true,
  port: 25565,
  onlineMode: true,
  whitelist: false,
  modCount: 23,
});

async function importServer(req: ImportRequest): Promise<ServerConfig> {
  await task(null, "import", 4210, 2600);
  await emit("provision://progress", { step: "installingJava", major: 17 });
  await sleep(600);
  await emit("provision://progress", { step: "finalizing" });
  await sleep(300);
  const id = uniqueId(req.name);
  const config = makeConfig({
    id,
    name: req.name,
    mcVersion: req.mcVersion ?? "1.20.4",
    loader: { kind: "fabric", loaderVersion: "0.15.11" },
    javaMajor: 17,
    memory: req.memory,
    cpu: req.cpu,
    port: req.port,
    eulaAccepted: req.eulaAccepted,
    launch: { kind: "jar", path: "fabric-server-launch.jar" },
  });
  servers = [...servers, summary(config)];
  properties.set(id, {
    exists: true,
    values: { "online-mode": "true", "white-list": "false", "server-port": `${req.port}` },
  });
  return config;
}

// ---------- Comandos ----------

mockIPC(
  async (cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    const id = a.id as string;
    switch (cmd) {
      // La app
      case "app_info":
        return {
          version: "0.3.1",
          dataDir: DATA_DIR,
          // Como la versión publicada, que trae su clave (salvo con ?nocf=1).
          curseforgeEnabled: curseforgeOn(),
          language: settings.language ?? systemLang,
          platform,
        };
      case "hardware_info":
        return { logicalCpus: 12, totalMemoryMb: 16384, recommendedMaxMb: 12288 };
      case "network_info":
        return { localIp: LOCAL_IP };
      case "list_java_runtimes":
        return [
          { major: 17, releaseName: "jdk-17.0.13+11", home: "", javaBin: "" },
          { major: 21, releaseName: "jdk-21.0.5+11", home: "", javaBin: "" },
        ];
      case "get_settings":
        return view();
      case "save_settings":
        settings = a.settings as AppSettings;
        return view();
      case "set_secret":
        await sleep(200);
        // Como Micra, la clave se prueba con CurseForge antes de guardarla.
        if (a.key === "curseforgeApiKey" && String(a.value).startsWith("cfc_pat_")) {
          throw "CurseForge no acepta esa clave: es la «Profile API key» (empieza por cfc_pat_). En console.curseforge.com abre «API keys», copia la clave que aparece allí y guárdala en Ajustes";
        }
        secrets[a.key as SecretKey] = true;
        return view();
      case "delete_secret":
        secrets[a.key as SecretKey] = false;
        return view();
      case "test_notification":
        await sleep(400);
        return null;
      case "check_app_update":
        await sleep(600);
        return params.get("update")
          ? {
              version: "0.3.2",
              notes: "Copias más rápidas y correcciones en el túnel.",
              date: new Date().toISOString(),
              canInstall: true,
            }
          : null;
      case "install_app_update":
        await sleep(2500);
        return null;

      // Contenido para crear servidores
      case "minecraft_versions":
        await sleep(300);
        return VERSIONS.map((v) => ({ id: v, type: "release" }));
      case "loader_game_versions":
        await sleep(300);
        return LOADER_VERSIONS[a.loader as string] ?? [];
      case "search_content":
        await sleep(350);
        return contentPage(a.query as ContentQuery);
      case "modrinth_versions": {
        await sleep(300);
        const pack = PACKS.find((p) => p[0] === a.projectId);
        const loaders = (a.loaders as string[] | null) ?? [];
        // Como el comando de Micra: solo las versiones de los loaders pedidos.
        return (pack ? packVersions(pack) : [])
          .filter(([, , loader]) => !loaders.length || loaders.includes(loader))
          .map(([v, mc, loader]) => ({
            id: `${a.projectId}-${v}`,
            name: v,
            version_number: v,
            version_type: "release",
            game_versions: [mc],
            loaders: [loader],
            date_published: "2026-09-01",
          }));
      }
      case "curseforge_files": {
        await sleep(300);
        const pack = CF_PACKS[Number(a.modId) - CF_PACK_IDS];
        return ["2.4.1", "2.4.0"].map((v, i) => ({
          id: 5000 + i,
          modId: a.modId,
          displayName: `Pack ${v}`,
          fileName: `pack-${v}.zip`,
          gameVersions: [pack?.[5] ?? "1.20.1", loaderName(pack?.[6] ?? "forge")],
          serverPackFileId: 6000 + i,
          isServerPack: false,
        }));
      }

      // Servidores
      case "list_servers":
        return servers;
      case "create_server":
        return provision(a.request as NewServerRequest);
      case "delete_server": {
        await sleep(700);
        const { status } = find(id);
        if (status !== "stopped" && status !== "crashed") throw "Apaga el servidor antes de hacer esto";
        servers = servers.filter((s) => s.config.id !== id);
        if (a.deleteBackups) backups.delete(id);
        return null;
      }
      case "start_server":
        start(id);
        return null;
      case "stop_server":
        await stop(id);
        return null;
      case "restart_server":
        await stop(id);
        start(id);
        return null;
      case "send_command":
        command(id, a.command as string);
        return null;
      case "update_server_settings":
        await sleep(250);
        return applySettings(id, a.settings as ServerSettings);

      // Jugadores
      case "player_lists":
        await sleep(150);
        return listsOf(id);
      case "player_action":
        await sleep(200);
        playerAction(id, a.action as PlayerAction, a.target as string);
        return null;

      // Copias
      case "list_backups":
        return backupsOf(id);
      case "create_backup": {
        const scope = a.scope as BackupScope;
        await task(id, "backup", scope === "full" ? 2860 : 1240, 2200);
        const info = backup(
          0,
          "manual",
          scope,
          scope === "full" ? 650_000_000 : 415_000_000,
          find(id).config.mcVersion,
        );
        backups.set(id, [info, ...backupsOf(id)]);
        return info;
      }
      case "restore_backup": {
        update(id, (s) => ({ ...s, busy: true }));
        const safety = backup(0, "preRestore", "world", 410_000_000, find(id).config.mcVersion);
        await task(id, "restore", 1240, 2400);
        backups.set(id, [safety, ...backupsOf(id)]);
        update(id, (s) => ({ ...s, busy: false }));
        return null;
      }
      case "delete_backup":
        backups.set(
          id,
          backupsOf(id).filter((b) => b.fileName !== a.name),
        );
        return null;

      // Mods y modpack
      case "list_mods":
        await sleep(150);
        return modsOf(id);
      case "set_mod_enabled": {
        const enabled = a.enabled as boolean;
        let renamed = "";
        mods.set(
          id,
          modsOf(id).map((m) => {
            if (m.fileName !== a.fileName) return m;
            renamed = enabled ? m.fileName.replace(/\.disabled$/, "") : `${m.fileName}.disabled`;
            return { ...m, enabled, fileName: renamed };
          }),
        );
        if (enabled) {
          updateCrash(id, (crash) => ({
            ...crash,
            missing: crash.missing.filter((m) => m.disabledFile !== a.fileName),
          }));
          forgetInstalled(id);
        }
        return renamed;
      }
      case "remove_mod": {
        mods.set(
          id,
          modsOf(id).filter((m) => m.fileName !== a.fileName),
        );
        // Si era uno de los que impedían arrancar, ya no se avisa de él ni de
        // lo que le faltaba solo a él.
        updateCrash(id, (crash) => ({
          ...crash,
          culprits: crash.culprits.filter((c) => c.fileName !== a.fileName),
          missing: crash.missing
            .map((m) => ({ ...m, requiredBy: m.requiredBy.filter((r) => r.fileName !== a.fileName) }))
            .filter((m, i) => m.requiredBy.length > 0 || crash.missing[i].requiredBy.length === 0),
        }));
        return null;
      }
      case "install_missing_mod": {
        await sleep(1100);
        const wanted = find(id).live.crash?.missing.find((m) => m.id === a.modId);
        if (!wanted?.found) throw `No se sabe dónde descargar ${a.modId}`;
        const file = `${wanted.id}-${modrinthLoadersOf(find(id).config.loader)[0]}-1.0.0.jar`;
        mods.set(id, [...modsOf(id), mod(file, wanted.found.title, "1.0.0", wanted.id, false)]);
        updateCrash(id, (crash) => ({ ...crash, missing: crash.missing.filter((m) => m.id !== a.modId) }));
        return [file];
      }
      case "dismiss_crash":
        setLive(id, { crash: null });
        return null;
      case "check_mod_updates": {
        await sleep(700);
        const list = remoteInfo(id);
        setLive(id, { modUpdates: list.filter((r) => r.update).length });
        return list;
      }
      case "update_mod": {
        await sleep(900);
        let fileName = "";
        mods.set(
          id,
          modsOf(id).map((m) => {
            if (m.fileName !== a.fileName || !m.modId) return m;
            const [version, file] = UPDATES[m.modId];
            fileName = file;
            return { ...m, fileName: file, version, sha1: fakeHash(file) };
          }),
        );
        setLive(id, { modUpdates: remoteInfo(id).filter((r) => r.update).length });
        return fileName;
      }
      case "search_mods": {
        await sleep(300);
        const { config } = find(id);
        return contentPage({
          query: String(a.query),
          kind: "mod",
          gameVersion: config.mcVersion,
          loaders: modrinthLoadersOf(config.loader),
          cursor: (a.cursor as SearchCursor | null) ?? null,
        });
      }
      case "install_mod": {
        await sleep(1100);
        const curseforge = a.source === "curseforge";
        const entry = curseforge
          ? CF_MOD_CATALOG.find(([cid]) => String(cid) === a.projectId)
          : MOD_CATALOG.find(([pid]) => pid === a.projectId);
        const name = entry?.[1] ?? String(a.projectId);
        if (params.get("modfail") && ++wizardInstalls === 2) {
          throw `${name} no tiene ninguna versión para Minecraft ${find(id).config.mcVersion}`;
        }
        const modId = curseforge ? slug(name) : String(a.projectId);
        const file = `${modId}-${modrinthLoadersOf(find(id).config.loader)[0]}-1.0.0.jar`;
        if (curseforge) fromCurseForge.set(fakeHash(file), String(a.projectId));
        mods.set(id, [...modsOf(id), mod(file, name, "1.0.0", modId, false)]);
        forgetInstalled(id);
        return [file];
      }
      case "scan_mods": {
        const list = modsOf(id);
        await task(id, "scan", list.length, 1400);
        const report: ScanReport = { scanned: list.length, files: [], host: [] };
        if (params.get("malware") && list.length) {
          report.files.push({
            path: `mods${SEP}${list[3].fileName}`,
            sha1: list[3].sha1,
            findings: [{ severity: "critical", rule: "fractureiser", location: "dev/neko/nekoclient/Client.class" }],
          });
        }
        return report;
      }
      case "quarantine_mod": {
        const name = String(a.path).split(/[\\/]/).pop();
        mods.set(
          id,
          modsOf(id).filter((m) => m.fileName !== name),
        );
        return null;
      }
      case "check_pack_update":
        await sleep(800);
        return find(id).live.packUpdate;
      case "update_pack": {
        const step = (payload: unknown) => emit("provision://progress", payload);
        update(id, (s) => ({ ...s, busy: true }));
        await step({ step: "fetchingContent" });
        await sleep(600);
        for (let done = 0; done <= 212; done += 53) {
          await step({ step: "downloadingFiles", done, total: 212 });
          await sleep(300);
        }
        await step({ step: "installingLoader" });
        await sleep(600);
        await step({ step: "finalizing" });
        await sleep(300);
        const s = find(id);
        const latest = s.live.packUpdate?.latest ?? "2.4.1";
        const config: ServerConfig = {
          ...s.config,
          source: { kind: "modrinth", projectId: "aventura-cosmica", versionId: `aventura-cosmica-${latest}` },
        };
        update(id, (x) => ({ ...x, config, busy: false }));
        backups.set(id, [backup(0, "preUpdate", "full", 2_700_000_000, config.mcVersion), ...backupsOf(id)]);
        setLive(id, { packUpdate: null });
        return config;
      }

      // server.properties
      case "get_properties":
        await sleep(120);
        return propertiesOf(id);
      case "save_properties": {
        await sleep(200);
        const changes = a.changes as Record<string, string>;
        const current = properties.get(id) ?? { exists: false, values: {} };
        const { rules } = propertiesOf(id);
        // Encendido, las reglas de juego cambian ya; el modo extremo, al arrancar.
        const running = find(id).status === "running";
        const pending = worldPending.get(id) ?? new Set<string>();
        for (const [key, value] of Object.entries(changes)) {
          if (key in rules && running) {
            pending.delete(key);
            log(id, `Gamerule ${rules[key]} is now set to: ${value}`);
          } else if (key in rules || key === "hardcore") pending.add(key);
        }
        worldPending.set(id, pending);
        properties.set(id, { exists: current.exists, values: { ...current.values, ...changes } });
        return propertiesOf(id);
      }

      // Estadísticas
      case "server_stats":
        await sleep(200);
        return history(id, a.range as StatsRange);

      // Red
      case "public_ip":
        await sleep(400);
        return PUBLIC_IP;
      case "check_reachable":
        await sleep(1500);
        return true;
      case "playit_claim_start":
        claimPolls = 0;
        return { code: "f3a9c2", url: "https://playit.gg/claim/f3a9c2" };
      case "playit_claim_poll": {
        claimPolls += 1;
        if (claimPolls < 2) return { status: "waitingForUserVisit", linked: false };
        if (claimPolls < 4) return { status: "waitingForUser", linked: false };
        secrets.playitAgent = true;
        return { status: "userAccepted", linked: true };
      }

      // Importar
      case "pick_folder":
        await sleep(300);
        return IMPORT_PATH;
      case "analyze_import":
        await sleep(900);
        return analysis(a.path as string);
      case "import_server":
        return importServer(a.request as ImportRequest);

      // Mapa web
      case "enable_map": {
        await sleep(1500);
        const accepted = a.acceptDownload as boolean;
        update(id, (s) => ({
          ...s,
          config: { ...s.config, map: { ...s.config.map, enabled: true, acceptedDownload: accepted } },
        }));
        setLive(id, { mapNeedsAccept: !accepted });
        return find(id).config;
      }
      case "disable_map":
        update(id, (s) => ({ ...s, config: { ...s.config, map: { ...s.config.map, enabled: false } } }));
        setLive(id, { mapNeedsAccept: false });
        return find(id).config;
      case "accept_map_download":
        update(id, (s) => ({ ...s, config: { ...s.config, map: { ...s.config.map, acceptedDownload: true } } }));
        setLive(id, { mapNeedsAccept: false });
        return find(id).config;

      default:
        // open_server_folder, open_backups_folder, open_data_folder, open_map, plugin:opener|open_url…
        console.info("[mock]", cmd, args);
        return null;
    }
  },
  { shouldMockEvents: true },
);

if (scenario === "running") {
  setTimeout(() => start("survival-con-amigos", true), 300);
}
