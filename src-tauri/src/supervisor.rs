//! Arranque, parada y vigilancia de cada servidor en marcha.
//!
//! Antes de arrancar: puerto libre, RAM suficiente, `server.properties` al
//! día, modo extremo escrito en el mundo, bloqueos vencidos levantados y
//! análisis de malware de los mods.
//! Mientras funciona: reenvía la consola, aplica las reglas de juego
//! pendientes, mide CPU, RAM y TPS, guarda las estadísticas, bloquea IPs
//! sospechosas, hace las copias automáticas y los reinicios programados,
//! abre el puerto o el túnel y avisa a la persona.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime};

use chrono::{Local, Utc};
use micra_core::net::upnp::{UpnpMapping, RENEW_EVERY};
use micra_core::notify::{restart_warning, NotifyEvent};
use micra_core::server::backup::{self, BackupKind};
use micra_core::server::config::ServerConfig;
use micra_core::server::diagnose::{self, Confidence, CrashInput, CrashReport};
use micra_core::server::guard::{Guard, GuardAction, GuardState};
use micra_core::server::monitor::ProcessMonitor;
use micra_core::server::pack::{self, PackManifest};
use micra_core::server::process::{message_of, ServerEvent, ServerHandle, ServerStatus};
use micra_core::server::schedule::{ScheduleAction, Scheduler};
use micra_core::server::stats::{self, MinuteAccumulator};
use micra_core::server::tps::{self, TpsReading};
use micra_core::server::{gamerules, map, missing, mods, players, ports, properties, scan, world};
use micra_core::{hardware, tr, Error, Result};
use tauri::async_runtime::JoinHandle;
use tauri::AppHandle;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, watch, Mutex};

use crate::events;
use crate::notifier::notify;
use crate::state::{LiveInfo, RunningServer, Shared, StopReason, TunnelInfo, UpnpInfo, WorldState};
use crate::tunnel;

/// Cada cuánto se miden los TPS.
const TPS_EVERY: Duration = Duration::from_secs(30);
/// Cada cuánto se buscan actualizaciones de mods y modpack.
const UPDATES_EVERY: Duration = Duration::from_secs(6 * 3600);
/// Cada cuánto se comprueba el agente del túnel (y la red en general).
const TUNNEL_CHECK: Duration = Duration::from_secs(60);
/// Cada cuánto se vuelve a preguntar a playit.gg por un túnel que ya funciona.
const TUNNEL_REFRESH: Duration = Duration::from_secs(600);
/// Líneas del final de la consola que se guardan para entender un fallo.
const TAIL_LINES: usize = 2000;
/// Lo que se espera a Modrinth para saber qué mods son solo de cliente.
const CLIENT_ONLY_LOOKUP: Duration = Duration::from_secs(15);
/// Lo que se espera a Modrinth y CurseForge para saber dónde descargar los
/// mods que faltan.
const MISSING_LOOKUP: Duration = Duration::from_secs(25);

/// Carpeta de un servidor existente. Los ids de Micra son siempre slugs
/// (`[a-z0-9-]`): se rechaza cualquier otra cosa (`..`, separadores, `C:`).
pub fn server_dir(state: &Shared, id: &str) -> Result<PathBuf> {
    let is_slug = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    let dir = state.paths.server_dir(id);
    if !is_slug || !dir.is_dir() {
        return Err(Error::NotFound(tr!("servidor {id}", "server {id}")));
    }
    Ok(dir)
}

pub async fn start(app: &AppHandle, state: &Shared, id: &str) -> Result<()> {
    let _busy = state.begin_busy(id)?;
    if state.is_alive(id).await {
        return Err(Error::AlreadyRunning);
    }
    let dir = server_dir(state, id)?;
    let mut config = ServerConfig::load(&dir).await?;
    if !config.eula_accepted {
        return Err(Error::InvalidConfig(tr!(
            "falta aceptar el EULA de Minecraft",
            "the Minecraft EULA has not been accepted"
        )));
    }
    check_ports(state, &config).await?;
    check_memory(state, &config).await?;
    let java = state.java.ensure(config.java_major, None).await?;
    prepare_properties(&dir, &config).await?;
    let world = prepare_world(&dir, &mut config).await;
    lift_expired_bans(&dir).await;

    let report = scan::scan_server(&dir, None).await?;
    if report.has_critical() {
        notify(
            app,
            state,
            NotifyEvent::Malware {
                server: config.name.clone(),
                files: report.critical_files().join(", "),
            },
        );
        return Err(scan::malware_error(&report));
    }
    if config.map.enabled {
        if let Err(e) = map::apply_config(&dir, &config).await {
            tracing::warn!("no se pudo ajustar BlueMap: {e}");
        }
    }

    let (handle, events) = ServerHandle::launch(&config, &dir, &java).await?;
    let (config_tx, _) = watch::channel(config);
    let (finished, _) = watch::channel(false);
    let server = Arc::new(RunningServer {
        handle: Arc::new(handle),
        dir,
        config: config_tx,
        started_at: Local::now(),
        stop_reason: StdMutex::new(None),
        finished,
        world: StdMutex::new(world),
    });
    state
        .running
        .write()
        .await
        .insert(id.to_string(), server.clone());
    let live = state.update_live(id, |l| {
        *l = LiveInfo {
            pack_update: l.pack_update.take(),
            mod_updates: l.mod_updates,
            sandbox: Some(server.handle.sandbox().clone()),
            ..LiveInfo::default()
        }
    });
    events::live(app, id, &live);
    tauri::async_runtime::spawn(supervise(app.clone(), state.clone(), server, events));
    Ok(())
}

pub async fn stop(state: &Shared, id: &str, reason: StopReason) -> Result<()> {
    let server = state.alive(id).await?;
    *server.stop_reason.lock().unwrap() = Some(reason);
    server.handle.stop(Duration::from_secs(60)).await
}

/// Otro servidor de Micra o cualquier otro programa usando el puerto.
async fn check_ports(state: &Shared, config: &ServerConfig) -> Result<()> {
    for (other_id, other) in state.running.read().await.iter() {
        if other_id == &config.id || !other.handle.is_alive() {
            continue;
        }
        let other = other.config.borrow();
        if other.port == config.port {
            return Err(Error::PortInUse {
                port: config.port,
                by: Some(other.name.clone()),
            });
        }
    }
    if !ports::is_port_free(config.port) {
        return Err(Error::PortInUse {
            port: config.port,
            by: None,
        });
    }
    Ok(())
}

/// La suma de la RAM de los servidores encendidos no puede pasar de la del
/// equipo: el sistema empezaría a cerrar programas.
async fn check_memory(state: &Shared, config: &ServerConfig) -> Result<()> {
    let others: u64 = state
        .running
        .read()
        .await
        .iter()
        .filter(|(id, s)| *id != &config.id && s.handle.is_alive())
        .map(|(_, s)| s.config.borrow().memory.max_mb)
        .sum();
    let total = hardware::hardware_info().total_memory_mb;
    let max = config.memory.max_mb;
    if others > 0 && others + max > total {
        return Err(Error::InvalidConfig(tr!(
            "no hay RAM para otro servidor: los encendidos ya usan {others} MB y este pide {max} MB de {total} MB. Apaga alguno o baja su RAM en Ajustes",
            "there is not enough RAM for another server: the running ones already use {others} MB and this one asks for {max} MB of {total} MB. Stop one or lower its RAM in Settings"
        )));
    }
    Ok(())
}

/// Puerto del juego al día, RCON con una contraseña fuerte si está activo y
/// el mundo dentro de la carpeta del servidor.
async fn prepare_properties(dir: &Path, config: &ServerConfig) -> Result<()> {
    let props = properties::read_properties(dir).await?;
    let mut changes: Vec<(&str, String)> = Vec::new();
    if props.get("server-port").map(|p| p.trim()) != Some(config.port.to_string().as_str()) {
        changes.push(("server-port", config.port.to_string()));
    }
    if properties::flag(&props, "enable-rcon", false)
        && properties::is_weak_password(props.get("rcon.password").map_or("", |p| p.as_str()))
    {
        changes.push(("rcon.password", properties::random_password()));
    }
    // Un mundo fuera de la carpeta del servidor (`level-name=../...`), puesto
    // por un pack, haría que el servidor escribiera fuera de ella.
    if props
        .get("level-name")
        .is_some_and(|name| !properties::is_safe_level_name(name.trim()))
    {
        tracing::warn!("level-name apuntaba fuera de la carpeta del servidor; se usa «world»");
        changes.push(("level-name", "world".to_string()));
    }
    if !changes.is_empty() {
        properties::set_properties(dir, &changes).await?;
    }
    Ok(())
}

/// Escribe en el mundo el modo extremo elegido con el servidor encendido o
/// apagado (`server.properties` solo cuenta al crearlo) y lee lo que el
/// mundo tiene guardado antes de que el servidor lo cargue.
async fn prepare_world(dir: &Path, config: &mut ServerConfig) -> WorldState {
    let props = properties::read_properties(dir).await.unwrap_or_default();
    let world_dir = world::world_dir(dir, &props);
    // Nunca se escribe a través de un enlace que un mod haya dejado en el camino.
    let pending = config.world.pending_hardcore.filter(|_| {
        micra_core::fsutil::check_no_links(dir, &world_dir.join("level.dat"))
            .inspect_err(|e| tracing::warn!("no se cambia el modo extremo: {e}"))
            .is_ok()
    });
    if let Some(hardcore) = pending {
        // Si el mundo aún no existe, lo crea server.properties.
        match world::set_hardcore(&world_dir, hardcore).await {
            Ok(_) => {
                config.world.pending_hardcore = None;
                if let Err(e) = save_world_tweaks(dir, |w| w.pending_hardcore = None).await {
                    tracing::warn!("no se pudo guardar la configuración: {e}");
                }
            }
            Err(e) => tracing::warn!("no se pudo cambiar el modo extremo del mundo: {e}"),
        }
    }
    let rules: Vec<&str> = gamerules::moved(&config.mc_version)
        .into_iter()
        .map(|(property, _)| property)
        .collect();
    WorldState {
        hardcore: world::read_hardcore(&world_dir).await,
        rules: world::read_rules(&world_dir, &rules).await,
    }
}

/// Cambia lo pendiente para el mundo en `micra.json`. La copia del servidor
/// en marcha no se toca: nada la lee y avisaría a las tareas sin motivo.
pub async fn save_world_tweaks(
    dir: &Path,
    change: impl FnOnce(&mut micra_core::server::config::WorldTweaks),
) -> Result<()> {
    let mut saved = ServerConfig::load(dir).await?;
    change(&mut saved.world);
    saved.save(dir).await
}

/// Cambia una regla de juego con el servidor encendido; `true` si respondió.
pub async fn set_rule(server: &RunningServer, property: &str, rule: &str, value: bool) -> bool {
    let confirmed = server
        .handle
        .command_and_wait(
            &gamerules::set_command(rule, value),
            |msg| gamerules::parse_set(msg).is_some_and(|(r, v)| r == rule && v == value),
            Duration::from_secs(5),
        )
        .await
        .unwrap_or(false);
    if confirmed {
        server
            .world
            .lock()
            .unwrap()
            .rules
            .insert(property.to_string(), value);
    }
    confirmed
}

/// Aplica las reglas guardadas con el servidor apagado y pregunta el valor
/// de todas: un mundo de una versión anterior aún no las tiene guardadas.
async fn sync_rules(server: &RunningServer) {
    let mc_version = server.config.borrow().mc_version.clone();
    let moved = gamerules::moved(&mc_version);
    if moved.is_empty() {
        return;
    }
    let pending = ServerConfig::load(&server.dir)
        .await
        .map(|c| c.world.pending_rules)
        .unwrap_or_default();
    let mut done = Vec::new();
    for (property, value) in &pending {
        // Una regla que esta versión no tiene ya no se podrá aplicar.
        let applied = match gamerules::rule_for(property, &mc_version) {
            Some(rule) => set_rule(server, property, rule, *value).await,
            None => true,
        };
        if applied {
            done.push(property.clone());
        }
    }
    if !done.is_empty() {
        let saved = save_world_tweaks(&server.dir, |w| {
            for property in &done {
                w.pending_rules.remove(property);
            }
        });
        if let Err(e) = saved.await {
            tracing::warn!("no se pudo guardar la configuración: {e}");
        }
    }

    let commands: Vec<String> = moved
        .iter()
        .map(|(_, rule)| gamerules::query_command(rule))
        .collect();
    let expected = commands.len();
    let Ok(lines) = server
        .handle
        .query_all(
            &commands,
            gamerules::is_query_line,
            move |lines| lines.len() >= expected,
            Duration::from_secs(3),
        )
        .await
    else {
        return;
    };
    let answers: BTreeMap<String, bool> = lines
        .iter()
        .filter_map(|line| gamerules::parse_query(line))
        .filter_map(|(rule, value)| Some((gamerules::property_of(&rule)?.to_string(), value)))
        .collect();
    server.world.lock().unwrap().rules.extend(answers);
}

/// Bloqueos temporales que vencieron con el servidor apagado.
async fn lift_expired_bans(dir: &Path) {
    let mut guard_state = GuardState::load(dir).await;
    let expired = guard_state.take_expired(Utc::now());
    if expired.is_empty() {
        return;
    }
    for ip in &expired {
        if let Err(e) = players::pardon_ip_offline(dir, &ip.to_string()).await {
            tracing::warn!("no se pudo desbloquear {ip}: {e}");
        }
    }
    if let Err(e) = guard_state.save(dir).await {
        tracing::warn!("no se pudo guardar el estado de la protección: {e}");
    }
}

/// Puerto abierto con UPnP mientras el servidor funciona.
type UpnpSlot = Arc<Mutex<Option<UpnpMapping>>>;

async fn supervise(
    app: AppHandle,
    state: Shared,
    server: Arc<RunningServer>,
    mut events: broadcast::Receiver<ServerEvent>,
) {
    let (id, name, dir) = {
        let cfg = server.config.borrow();
        (cfg.id.clone(), cfg.name.clone(), server.dir.clone())
    };
    let handle = server.handle.clone();
    let mut config_rx = server.config.subscribe();
    let guard_policy = config_rx.borrow().security.guard.clone();
    let mut guard = Guard::new(guard_policy, GuardState::load(&dir).await);
    let mut scheduler: Option<Scheduler<Local>> = None;
    let mut minute = MinuteAccumulator::default();
    let stats_dir = state.paths.stats_dir(&id);
    let mut monitor = handle.pid().map(ProcessMonitor::new);
    let upnp: UpnpSlot = Arc::new(Mutex::new(None));
    let mut background: Vec<JoinHandle<()>> = Vec::new();
    let mut ready = false;
    let mut crashed = false;
    // El final de la consola, para saber qué falló si se cae.
    let mut tail: VecDeque<String> = VecDeque::with_capacity(TAIL_LINES);
    let mut lagged = false;
    let mut fatal_seen = false;

    let mut second = tokio::time::interval(Duration::from_secs(1));
    second.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            received = events.recv() => {
                let event = match received {
                    Ok(event) => event,
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "la consola va más rápido que la interfaz");
                        lagged = true;
                        continue;
                    }
                    Err(RecvError::Closed) => break,
                };
                events::server_event(&app, &id, &event);
                match &event {
                    ServerEvent::Log { line, .. } => {
                        if tail.len() == TAIL_LINES {
                            tail.pop_front();
                        }
                        tail.push_back(line.clone());
                        let msg = message_of(line);
                        fatal_seen |= !ready && diagnose::stops_the_start(msg);
                        // Reglas cambiadas en la consola o por un operador en el juego.
                        if let Some((rule, value)) = gamerules::parse_set(msg) {
                            if let Some(property) = gamerules::property_of(&rule) {
                                server.world.lock().unwrap().rules.insert(property.to_string(), value);
                            }
                        }
                        let actions = guard.on_line(msg, Instant::now(), Utc::now());
                        if !actions.is_empty() {
                            run_guard_actions(&app, &state, &handle, &name, actions);
                            save_guard(&dir, &guard.state);
                        }
                        if map::is_accept_prompt(msg) && !config_rx.borrow().map.accepted_download {
                            let live = state.update_live(&id, |l| l.map_needs_accept = true);
                            events::live(&app, &id, &live);
                        }
                    }
                    ServerEvent::Status { status: ServerStatus::Running } if !ready => {
                        ready = true;
                        let cfg = config_rx.borrow().clone();
                        scheduler = Some(new_scheduler(&app, &state, &cfg));
                        notify(&app, &state, NotifyEvent::ServerReady { server: name.clone() });
                        background.push(spawn_ready_tasks(&app, &state, &server));
                        background.push(spawn_tps(&app, &state, &server));
                        background.push(spawn_backups(&app, &state, &server));
                        background.push(spawn_updates(&app, &state, &server));
                        background.push(spawn_network(&app, &state, &server, upnp.clone()));
                    }
                    ServerEvent::Status { status } if matches!(status, ServerStatus::Stopped | ServerStatus::Crashed) => {
                        crashed = *status == ServerStatus::Crashed;
                        break;
                    }
                    ServerEvent::PlayerJoined { name: player } => notify(
                        &app,
                        &state,
                        NotifyEvent::PlayerJoined { server: name.clone(), player: player.clone() },
                    ),
                    ServerEvent::PlayerLeft { name: player } => notify(
                        &app,
                        &state,
                        NotifyEvent::PlayerLeft { server: name.clone(), player: player.clone() },
                    ),
                    _ => {}
                }
            }
            _ = second.tick() => {
                let players = handle.players().len();
                if let Some(sample) = monitor.as_mut().and_then(ProcessMonitor::sample) {
                    let tps = state.live(&id).tps;
                    events::stats(&app, &id, &sample, players, tps);
                    if let Some(point) = minute.push(Utc::now(), sample.cpu_percent, sample.memory_mb, players as u32, tps) {
                        save_point(&stats_dir, point);
                    }
                }
                let pardons = guard.tick(Utc::now());
                if !pardons.is_empty() {
                    run_guard_actions(&app, &state, &handle, &name, pardons);
                    save_guard(&dir, &guard.state);
                }
                if let Some(action) = scheduler.as_mut().and_then(|s| s.tick(Local::now(), players)) {
                    run_schedule(&state, &server, action).await;
                }
            }
            changed = config_rx.changed() => {
                if changed.is_err() {
                    continue;
                }
                let cfg = config_rx.borrow_and_update().clone();
                guard = Guard::new(cfg.security.guard.clone(), std::mem::take(&mut guard.state));
                if ready {
                    scheduler = Some(new_scheduler(&app, &state, &cfg));
                }
            }
        }
    }

    // El servidor se apagó: recoger.
    for task in background {
        task.abort();
    }
    if let Some(point) = minute.flush() {
        save_point(&stats_dir, point);
    }
    if let Err(e) = guard.state.save(&dir).await {
        tracing::warn!("no se pudo guardar el estado de la protección: {e}");
    }
    if let Some(mapping) = upnp.lock().await.take() {
        if let Err(e) = mapping.close().await {
            tracing::warn!("{e}");
        }
    }
    tunnel::release(&state).await;
    let live = state.update_live(&id, |l| {
        l.tps = None;
        l.upnp = None;
        l.tunnel = None;
        l.restart_at = None;
    });
    events::live(&app, &id, &live);

    let reason = *server.stop_reason.lock().unwrap();
    let failed = crashed && reason.is_none();
    // También si se quedó colgado tras el error y la persona lo paró.
    if failed || (fatal_seen && !ready && reason == Some(StopReason::User)) {
        let console = if lagged {
            diagnose::log_tail(&dir, TAIL_LINES).await
        } else {
            Vec::from(tail)
        };
        let report = diagnose_crash(&app, &state, &server, console, !ready).await;
        if failed {
            notify(
                &app,
                &state,
                NotifyEvent::ServerCrashed {
                    server: name.clone(),
                    mods: report.as_ref().map(culprit_names).unwrap_or_default(),
                },
            );
        }
    }
    server.finished.send_replace(true);

    if reason == Some(StopReason::Restart) {
        if let Err(e) = start_again(app.clone(), state.clone(), id.clone()).await {
            events::error(
                &app,
                &tr!(
                    "«{name}» no pudo volver a arrancar",
                    "\"{name}\" could not start again"
                ),
                &e.to_string(),
            );
        }
    }
}

/// Lee por qué falló un servidor con mods y lo deja en el panel: primero lo
/// que dice el error y después, si Modrinth y CurseForge responden, dónde
/// descargar los mods que faltan y qué mods solo funcionan en el juego.
async fn diagnose_crash(
    app: &AppHandle,
    state: &Shared,
    server: &RunningServer,
    console: Vec<String>,
    during_start: bool,
) -> Option<CrashReport> {
    let (id, loader) = {
        let cfg = server.config.borrow();
        if !cfg.loader.supports_mods() {
            return None;
        }
        (cfg.id.clone(), cfg.loader.clone())
    };
    let dir = server.dir.clone();
    let mods = mods::list_mods(&dir).await.ok()?;
    // Margen por si el reloj de los archivos va por segundos.
    let since = SystemTime::from(server.started_at) - Duration::from_secs(2);
    let saved = diagnose::latest_report(&dir, since).await;
    let mut report = {
        let (mods, dir) = (mods.clone(), dir.clone());
        tauri::async_runtime::spawn_blocking(move || {
            diagnose::diagnose(&CrashInput {
                console: &console,
                report: saved
                    .as_ref()
                    .map(|(text, name)| (text.as_str(), name.as_str())),
                mods: &mods,
                server_dir: &dir,
                during_start,
                loader: &loader,
            })
        })
        .await
        .ok()?
    };
    diagnose::add_client_only(&mut report, &mods, &HashSet::new());
    // Después, por internet: dónde descargar los mods que faltan y, si el
    // error es de clases del juego (cliente) y no se sabe qué mod las pidió,
    // qué mods dice Modrinth que solo funcionan en el juego.
    let client_classes = report.client_classes;
    let wanted = report.missing.clone();
    let to_find = wanted.iter().any(|m| m.disabled_file.is_none());
    report.missing_checked = !to_find;
    let live = state.update_live(&id, |l| l.crash = Some(report.clone()));
    events::live(app, &id, &live);
    if !client_classes && !to_find {
        return Some(report);
    }
    let cfg = server.config.borrow().clone();
    let (app, state) = (app.clone(), state.clone());
    tauri::async_runtime::spawn(async move {
        let curseforge = state.curseforge().await;
        let client_only = async {
            if !client_classes {
                return HashSet::new();
            }
            let lookup = diagnose::client_only_on_modrinth(&state.modrinth, &mods);
            match tokio::time::timeout(CLIENT_ONLY_LOOKUP, lookup).await {
                Ok(Ok(sha1s)) => sha1s,
                _ => HashSet::new(),
            }
        };
        let found = async {
            let lookup =
                missing::find(&state.modrinth, curseforge.as_deref(), &cfg, &mods, &wanted);
            tokio::time::timeout(MISSING_LOOKUP, lookup)
                .await
                .unwrap_or_default()
        };
        let (sha1s, found) = tokio::join!(client_only, found);
        // Sin los mods que la persona ya quitó mientras tanto.
        let present: Vec<_> = mods
            .iter()
            .filter(|m| dir.join(mods::MODS_DIR).join(&m.file_name).exists())
            .cloned()
            .collect();
        let live = state.update_live(&id, |l| {
            let Some(crash) = l.crash.as_mut() else {
                return;
            };
            if !sha1s.is_empty() {
                diagnose::add_client_only(crash, &present, &sha1s);
            }
            for (mod_id, project) in found {
                if let Some(m) = crash.missing.iter_mut().find(|m| m.id == mod_id) {
                    m.found = Some(project);
                }
            }
            crash.missing_checked = true;
        });
        events::live(&app, &id, &live);
    });
    Some(report)
}

/// Los mods que el error señala con seguridad, para el aviso.
fn culprit_names(report: &CrashReport) -> String {
    report
        .culprits
        .iter()
        .filter(|c| c.confidence != Confidence::Possible)
        .take(3)
        .map(|c| c.name.clone().unwrap_or_else(|| c.file_name.clone()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `start` lanza la vigilancia y la vigilancia vuelve a llamar a `start` al
/// reiniciar: el tipo explícito corta ese ciclo para el compilador.
fn start_again(
    app: AppHandle,
    state: Shared,
    id: String,
) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
    Box::pin(async move { start(&app, &state, &id).await })
}

fn save_point(dir: &Path, point: stats::StatPoint) {
    let dir = dir.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = stats::append(&dir, &point) {
            tracing::warn!("no se pudo guardar la estadística: {e}");
        }
        let _ = stats::prune(&dir, Utc::now());
    });
}

fn save_guard(dir: &Path, guard_state: &GuardState) {
    let dir = dir.to_owned();
    let guard_state = guard_state.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = guard_state.save(&dir).await {
            tracing::warn!("no se pudo guardar el estado de la protección: {e}");
        }
    });
}

fn run_guard_actions(
    app: &AppHandle,
    state: &Shared,
    handle: &Arc<ServerHandle>,
    server: &str,
    actions: Vec<GuardAction>,
) {
    for action in actions {
        let command = match &action {
            GuardAction::BanIp { ip, attempts, .. } => {
                notify(
                    app,
                    state,
                    NotifyEvent::IpBlocked {
                        server: server.to_string(),
                        ip: ip.to_string(),
                        attempts: *attempts,
                    },
                );
                format!("ban-ip {ip} Too many failed login attempts")
            }
            GuardAction::PardonIp { ip } => format!("pardon-ip {ip}"),
            GuardAction::Kick { player, ip } => {
                notify(
                    app,
                    state,
                    NotifyEvent::PlayerKicked {
                        server: server.to_string(),
                        player: player.clone(),
                        ip: ip.to_string(),
                    },
                );
                format!("kick {player} Too many players from your network")
            }
        };
        let handle = handle.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = handle.send_command(&command).await {
                tracing::warn!("no se pudo aplicar la protección ({command}): {e}");
            }
        });
    }
}

fn new_scheduler(app: &AppHandle, state: &Shared, cfg: &ServerConfig) -> Scheduler<Local> {
    let scheduler = Scheduler::new(&cfg.schedule, Local::now());
    let restart_at = scheduler.restart_at().cloned();
    let live = state.update_live(&cfg.id, |l| l.restart_at = restart_at);
    events::live(app, &cfg.id, &live);
    scheduler
}

async fn run_schedule(state: &Shared, server: &Arc<RunningServer>, action: ScheduleAction) {
    match action {
        ScheduleAction::Warn { seconds } => {
            let text = restart_warning(state.lang().await, seconds);
            if let Err(e) = server.handle.send_command(&format!("say {text}")).await {
                tracing::warn!("no se pudo avisar del reinicio: {e}");
            }
        }
        ScheduleAction::Restart | ScheduleAction::IdleStop => {
            let reason = if action == ScheduleAction::Restart {
                StopReason::Restart
            } else {
                StopReason::Idle
            };
            *server.stop_reason.lock().unwrap() = Some(reason);
            let handle = server.handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = handle.stop(Duration::from_secs(60)).await {
                    tracing::warn!("no se pudo apagar el servidor: {e}");
                }
            });
        }
    }
}

/// Lo que se hace una vez cuando el servidor termina de arrancar.
fn spawn_ready_tasks(
    app: &AppHandle,
    state: &Shared,
    server: &Arc<RunningServer>,
) -> JoinHandle<()> {
    let (app, state, server) = (app.clone(), state.clone(), server.clone());
    tauri::async_runtime::spawn(async move {
        let cfg = server.config.borrow().clone();
        // Dueño elegido en el asistente cuyo perfil no se pudo consultar.
        if let Some(owner) = cfg.pending_owner.clone() {
            let mut ok = true;
            for command in [format!("whitelist add {owner}"), format!("op {owner}")] {
                ok &= server.handle.send_command(&command).await.is_ok();
            }
            if ok {
                if let Err(e) = clear_pending_owner(&server).await {
                    tracing::warn!("no se pudo guardar la configuración: {e}");
                }
            }
        }
        // BlueMap crea su configuración en el primer arranque.
        if cfg.map.enabled {
            match map::apply_config(&server.dir, &cfg).await {
                Ok(true) => {
                    let _ = server.handle.send_command("bluemap reload").await;
                    let live = state.update_live(&cfg.id, |l| l.map_needs_accept = false);
                    events::live(&app, &cfg.id, &live);
                }
                Ok(false) => {}
                Err(e) => tracing::warn!("no se pudo ajustar BlueMap: {e}"),
            }
        }
        sync_rules(&server).await;
    })
}

async fn clear_pending_owner(server: &RunningServer) -> Result<()> {
    let mut saved = ServerConfig::load(&server.dir).await?;
    saved.pending_owner = None;
    saved.save(&server.dir).await?;
    server.config.send_modify(|c| c.pending_owner = None);
    Ok(())
}

fn spawn_tps(app: &AppHandle, state: &Shared, server: &Arc<RunningServer>) -> JoinHandle<()> {
    let (app, state, server) = (app.clone(), state.clone(), server.clone());
    tauri::async_runtime::spawn(async move {
        let (id, command) = {
            let cfg = server.config.borrow();
            (
                cfg.id.clone(),
                tps::tps_command(&cfg.loader, &cfg.mc_version),
            )
        };
        let Some(command) = command else {
            return;
        };
        loop {
            tokio::time::sleep(TPS_EVERY).await;
            if !server.handle.is_alive() {
                return;
            }
            if server.handle.status() != ServerStatus::Running {
                continue;
            }
            let lines = match server
                .handle
                .query(command, tps::is_tps_line, Duration::from_secs(2))
                .await
            {
                Ok(lines) => lines,
                Err(_) => continue,
            };
            match tps::parse_tps(&lines) {
                TpsReading::Tps(value) => {
                    let live = state.update_live(&id, |l| l.tps = Some(value));
                    events::live(&app, &id, &live);
                }
                TpsReading::Unsupported => return,
                TpsReading::Unknown => {}
            }
        }
    })
}

fn spawn_backups(app: &AppHandle, state: &Shared, server: &Arc<RunningServer>) -> JoinHandle<()> {
    let (app, state, server) = (app.clone(), state.clone(), server.clone());
    tauri::async_runtime::spawn(async move {
        let mut config_rx = server.config.subscribe();
        loop {
            let policy = config_rx.borrow_and_update().backups.clone();
            if !policy.enabled {
                if config_rx.changed().await.is_err() {
                    return;
                }
                continue;
            }
            let wait = Duration::from_secs(u64::from(policy.interval_hours.max(1)) * 3600);
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                changed = config_rx.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    // Nuevo intervalo: volver a contar.
                    continue;
                }
            }
            if !server.handle.is_alive() {
                return;
            }
            let cfg = server.config.borrow().clone();
            let backups_dir = state.paths.backups_dir(&cfg.id);
            let result = backup::create_live(
                &server.handle,
                &server.dir,
                &backups_dir,
                &cfg,
                BackupKind::Auto,
                None,
            )
            .await;
            let error = match result {
                Ok(_) => backup::prune(&backups_dir, cfg.backups.keep).await.err(),
                Err(e) => Some(e),
            };
            let live = state.update_live(&cfg.id, |l| {
                l.last_backup_error = error.as_ref().map(ToString::to_string)
            });
            events::live(&app, &cfg.id, &live);
            if let Some(e) = error {
                notify(
                    &app,
                    &state,
                    NotifyEvent::BackupFailed {
                        server: cfg.name.clone(),
                        error: e.to_string(),
                    },
                );
            }
        }
    })
}

/// Busca actualizaciones del modpack y de los mods al arrancar y cada 6 h.
fn spawn_updates(app: &AppHandle, state: &Shared, server: &Arc<RunningServer>) -> JoinHandle<()> {
    let (app, state, server) = (app.clone(), state.clone(), server.clone());
    tauri::async_runtime::spawn(async move {
        loop {
            let cfg = server.config.borrow().clone();
            check_updates(&app, &state, &server.dir, &cfg).await;
            tokio::time::sleep(UPDATES_EVERY).await;
            if !server.handle.is_alive() {
                return;
            }
        }
    })
}

pub async fn check_updates(app: &AppHandle, state: &Shared, dir: &Path, cfg: &ServerConfig) {
    let curseforge = state.curseforge().await;
    let pack_update = if cfg.source.is_some() {
        let manifest = PackManifest::load(dir).await;
        pack::check_update(
            &state.modrinth,
            curseforge.as_deref(),
            cfg,
            manifest.as_ref(),
        )
        .await
        .unwrap_or_else(|e| {
            tracing::warn!("no se pudo buscar una versión nueva del modpack: {e}");
            None
        })
    } else {
        None
    };
    let mut mod_updates = 0;
    if cfg.loader.supports_mods() {
        if let Ok(list) = mods::list_mods(dir).await {
            if !list.is_empty() {
                let sources = mods::Sources {
                    http: &state.http,
                    modrinth: &state.modrinth,
                    curseforge: curseforge.as_deref(),
                };
                match mods::check_updates(sources, dir, cfg, &list).await {
                    Ok(remote) => {
                        mod_updates = remote.iter().filter(|m| m.update.is_some()).count()
                    }
                    Err(e) => tracing::warn!("no se pudo buscar actualizaciones de mods: {e}"),
                }
            }
        }
    }
    let live = state.update_live(&cfg.id, |l| {
        l.pack_update = pack_update;
        l.mod_updates = mod_updates;
    });
    events::live(app, &cfg.id, &live);
}

/// UPnP y túnel según la configuración, también si cambia en marcha.
fn spawn_network(
    app: &AppHandle,
    state: &Shared,
    server: &Arc<RunningServer>,
    upnp: UpnpSlot,
) -> JoinHandle<()> {
    let (app, state, server) = (app.clone(), state.clone(), server.clone());
    tauri::async_runtime::spawn(async move {
        let mut config_rx = server.config.subscribe();
        let mut last_upnp_renew = tokio::time::Instant::now();
        let mut last_tunnel_refresh = tokio::time::Instant::now();
        let mut tunnel_on = false;
        loop {
            let cfg = config_rx.borrow_and_update().clone();
            let id = cfg.id.clone();

            // UPnP: abrir o cerrar según el ajuste; renovar la concesión.
            let mut slot = upnp.lock().await;
            match (cfg.network.upnp, slot.is_some()) {
                (true, false) => {
                    let live = state.update_live(&id, |l| l.upnp = Some(UpnpInfo::Opening));
                    events::live(&app, &id, &live);
                    let info = match UpnpMapping::open(cfg.port, &cfg.name).await {
                        Ok(mapping) => {
                            let info = UpnpInfo::Open {
                                mapping: mapping.info.clone(),
                            };
                            *slot = Some(mapping);
                            last_upnp_renew = tokio::time::Instant::now();
                            info
                        }
                        Err(e) => UpnpInfo::Failed {
                            error: e.to_string(),
                        },
                    };
                    let live = state.update_live(&id, |l| l.upnp = Some(info));
                    events::live(&app, &id, &live);
                }
                (false, true) => {
                    if let Some(mapping) = slot.take() {
                        if let Err(e) = mapping.close().await {
                            tracing::warn!("{e}");
                        }
                    }
                    let live = state.update_live(&id, |l| l.upnp = None);
                    events::live(&app, &id, &live);
                }
                (true, true) if last_upnp_renew.elapsed() >= RENEW_EVERY => {
                    if let Some(mapping) = slot.as_ref() {
                        if let Err(e) = mapping.renew().await {
                            tracing::warn!("no se pudo renovar el puerto: {e}");
                        }
                    }
                    last_upnp_renew = tokio::time::Instant::now();
                }
                _ => {}
            }
            drop(slot);

            // Túnel: preparar al activarlo; comprobar el agente cada minuto.
            if cfg.network.tunnel && !tunnel_on {
                tunnel_on = true;
                let live = state.update_live(&id, |l| l.tunnel = Some(TunnelInfo::Starting));
                events::live(&app, &id, &live);
                let info = tunnel::enable(&state, &id, cfg.port).await;
                last_tunnel_refresh = tokio::time::Instant::now();
                let live = state.update_live(&id, |l| l.tunnel = Some(info));
                events::live(&app, &id, &live);
            } else if !cfg.network.tunnel && tunnel_on {
                tunnel_on = false;
                tunnel::release(&state).await;
                let live = state.update_live(&id, |l| l.tunnel = None);
                events::live(&app, &id, &live);
            } else if tunnel_on {
                let ready = matches!(state.live(&id).tunnel, Some(TunnelInfo::Ready { .. }));
                if !ready || last_tunnel_refresh.elapsed() >= TUNNEL_REFRESH {
                    last_tunnel_refresh = tokio::time::Instant::now();
                    let info = tunnel::refresh(&state, &id, cfg.port).await;
                    let live = state.update_live(&id, |l| l.tunnel = Some(info));
                    events::live(&app, &id, &live);
                } else if let Err(e) = tunnel::restart_if_down(&state).await {
                    let info = TunnelInfo::Failed {
                        error: e.to_string(),
                    };
                    let live = state.update_live(&id, |l| l.tunnel = Some(info));
                    events::live(&app, &id, &live);
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(TUNNEL_CHECK) => {}
                changed = config_rx.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
            }
            if !server.handle.is_alive() {
                return;
            }
        }
    })
}
