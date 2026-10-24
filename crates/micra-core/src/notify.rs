//! Avisos: textos en los cinco idiomas de la app y envío a Discord.
//!
//! Las notificaciones de escritorio las muestra la capa de Tauri; aquí están
//! el texto de cada aviso, qué ajuste lo controla y el webhook de Discord.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::i18n::Lang;
use crate::settings::NotificationSettings;
use crate::{tr, Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NotifyEvent {
    PlayerJoined {
        server: String,
        player: String,
    },
    PlayerLeft {
        server: String,
        player: String,
    },
    ServerReady {
        server: String,
    },
    ServerCrashed {
        server: String,
        /// Mods que señala el error, separados por comas (vacío si ninguno).
        mods: String,
    },
    IpBlocked {
        server: String,
        ip: String,
        attempts: usize,
    },
    PlayerKicked {
        server: String,
        player: String,
        ip: String,
    },
    Malware {
        server: String,
        files: String,
    },
    BackupFailed {
        server: String,
        error: String,
    },
    Test,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NotifyText {
    pub title: String,
    pub body: String,
}

/// Color de la barra lateral del aviso en Discord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Good,
    Warning,
    Critical,
}

impl Tone {
    fn color(self) -> u32 {
        match self {
            Tone::Info => 0x3987e5,
            Tone::Good => 0x2fd27a,
            Tone::Warning => 0xf5b13d,
            Tone::Critical => 0xf0565a,
        }
    }
}

impl NotifyEvent {
    /// Si los ajustes permiten este aviso.
    pub fn enabled_in(&self, s: &NotificationSettings) -> bool {
        match self {
            NotifyEvent::PlayerJoined { .. } => s.player_join,
            NotifyEvent::PlayerLeft { .. } => s.player_leave,
            NotifyEvent::ServerReady { .. } => s.server_ready,
            NotifyEvent::ServerCrashed { .. } => s.server_crash,
            NotifyEvent::IpBlocked { .. }
            | NotifyEvent::PlayerKicked { .. }
            | NotifyEvent::Malware { .. } => s.security,
            NotifyEvent::BackupFailed { .. } => s.backup_failed,
            NotifyEvent::Test => true,
        }
    }

    pub fn tone(&self) -> Tone {
        match self {
            NotifyEvent::PlayerJoined { .. } | NotifyEvent::ServerReady { .. } => Tone::Good,
            NotifyEvent::PlayerLeft { .. } | NotifyEvent::Test => Tone::Info,
            NotifyEvent::IpBlocked { .. }
            | NotifyEvent::PlayerKicked { .. }
            | NotifyEvent::BackupFailed { .. } => Tone::Warning,
            NotifyEvent::ServerCrashed { .. } | NotifyEvent::Malware { .. } => Tone::Critical,
        }
    }

    pub fn text(&self, lang: Lang) -> NotifyText {
        use Lang::*;
        let (title, body) = match self {
            NotifyEvent::PlayerJoined { server, player } => match lang {
                Es => (format!("{player} ha entrado"), format!("Servidor: {server}")),
                En => (format!("{player} joined"), format!("Server: {server}")),
                Pt => (format!("{player} entrou"), format!("Servidor: {server}")),
                Fr => (format!("{player} a rejoint la partie"), format!("Serveur : {server}")),
                De => (format!("{player} ist beigetreten"), format!("Server: {server}")),
            },
            NotifyEvent::PlayerLeft { server, player } => match lang {
                Es => (format!("{player} ha salido"), format!("Servidor: {server}")),
                En => (format!("{player} left"), format!("Server: {server}")),
                Pt => (format!("{player} saiu"), format!("Servidor: {server}")),
                Fr => (format!("{player} a quitté la partie"), format!("Serveur : {server}")),
                De => (format!("{player} hat das Spiel verlassen"), format!("Server: {server}")),
            },
            NotifyEvent::ServerReady { server } => match lang {
                Es => (format!("{server} está en línea"), "Ya se puede entrar.".into()),
                En => (format!("{server} is online"), "Players can join now.".into()),
                Pt => (format!("{server} está online"), "Já é possível entrar.".into()),
                Fr => (format!("{server} est en ligne"), "Les joueurs peuvent se connecter.".into()),
                De => (format!("{server} ist online"), "Spieler können jetzt beitreten.".into()),
            },
            NotifyEvent::ServerCrashed { server, mods } if mods.is_empty() => match lang {
                Es => (
                    format!("{server} se ha detenido por un error"),
                    "Revisa la consola para ver qué pasó.".into(),
                ),
                En => (
                    format!("{server} stopped because of an error"),
                    "Check the console to see what happened.".into(),
                ),
                Pt => (
                    format!("{server} parou por causa de um erro"),
                    "Confira o console para ver o que aconteceu.".into(),
                ),
                Fr => (
                    format!("{server} s'est arrêté à cause d'une erreur"),
                    "Consultez la console pour voir ce qui s'est passé.".into(),
                ),
                De => (
                    format!("{server} wurde wegen eines Fehlers beendet"),
                    "Sieh in der Konsole nach, was passiert ist.".into(),
                ),
            },
            NotifyEvent::ServerCrashed { server, mods } => match lang {
                Es => (
                    format!("{server} se ha detenido por un error"),
                    format!("Causa probable: {mods}. Abre Micra para arreglarlo."),
                ),
                En => (
                    format!("{server} stopped because of an error"),
                    format!("Likely cause: {mods}. Open Micra to fix it."),
                ),
                Pt => (
                    format!("{server} parou por causa de um erro"),
                    format!("Causa provável: {mods}. Abra o Micra para resolver."),
                ),
                Fr => (
                    format!("{server} s'est arrêté à cause d'une erreur"),
                    format!("Cause probable : {mods}. Ouvrez Micra pour corriger le problème."),
                ),
                De => (
                    format!("{server} wurde wegen eines Fehlers beendet"),
                    format!("Wahrscheinliche Ursache: {mods}. Öffne Micra, um es zu beheben."),
                ),
            },
            NotifyEvent::IpBlocked {
                server,
                ip,
                attempts,
            } => match lang {
                Es => (
                    format!("IP bloqueada en {server}"),
                    format!("{ip} falló {attempts} intentos de entrada seguidos."),
                ),
                En => (
                    format!("IP blocked on {server}"),
                    format!("{ip} failed {attempts} join attempts in a row."),
                ),
                Pt => (
                    format!("IP bloqueado em {server}"),
                    format!("{ip} falhou {attempts} tentativas de entrada seguidas."),
                ),
                Fr => (
                    format!("IP bloquée sur {server}"),
                    format!("{ip} a échoué {attempts} tentatives de connexion d'affilée."),
                ),
                De => (
                    format!("IP auf {server} gesperrt"),
                    format!("{ip} ist {attempts}-mal hintereinander am Beitreten gescheitert."),
                ),
            },
            NotifyEvent::PlayerKicked { server, player, ip } => match lang {
                Es => (
                    format!("Límite de conexiones en {server}"),
                    format!("{player} fue expulsado: demasiados jugadores desde {ip}."),
                ),
                En => (
                    format!("Connection limit on {server}"),
                    format!("{player} was kicked: too many players from {ip}."),
                ),
                Pt => (
                    format!("Limite de conexões em {server}"),
                    format!("{player} foi expulso: jogadores demais a partir de {ip}."),
                ),
                Fr => (
                    format!("Limite de connexions sur {server}"),
                    format!("{player} a été expulsé : trop de joueurs depuis {ip}."),
                ),
                De => (
                    format!("Verbindungslimit auf {server}"),
                    format!("{player} wurde gekickt: zu viele Spieler von {ip}."),
                ),
            },
            NotifyEvent::Malware { server, files } => match lang {
                Es => (
                    format!("Malware detectado en {server}"),
                    format!("Archivos: {files}. El servidor no arrancará hasta que los pongas en cuarentena."),
                ),
                En => (
                    format!("Malware found on {server}"),
                    format!("Files: {files}. The server will not start until you quarantine them."),
                ),
                Pt => (
                    format!("Malware detectado em {server}"),
                    format!("Arquivos: {files}. O servidor não vai iniciar até você colocá-los em quarentena."),
                ),
                Fr => (
                    format!("Logiciel malveillant détecté sur {server}"),
                    format!("Fichiers : {files}. Le serveur ne démarrera pas tant qu'ils ne seront pas en quarantaine."),
                ),
                De => (
                    format!("Schadsoftware auf {server} gefunden"),
                    format!("Dateien: {files}. Der Server startet erst, wenn du sie in Quarantäne verschiebst."),
                ),
            },
            NotifyEvent::BackupFailed { server, error } => match lang {
                Es => (format!("La copia de seguridad de {server} falló"), error.clone()),
                En => (format!("{server} backup failed"), error.clone()),
                Pt => (format!("O backup de {server} falhou"), error.clone()),
                Fr => (format!("La sauvegarde de {server} a échoué"), error.clone()),
                De => (format!("Sicherung von {server} fehlgeschlagen"), error.clone()),
            },
            NotifyEvent::Test => match lang {
                Es => ("Prueba de Micra".into(), "Si ves esto, los avisos funcionan.".into()),
                En => ("Micra test".into(), "If you can see this, notifications work.".into()),
                Pt => ("Teste do Micra".into(), "Se você está vendo isto, os avisos funcionam.".into()),
                Fr => ("Test de Micra".into(), "Si vous voyez ceci, les notifications fonctionnent.".into()),
                De => ("Micra-Test".into(), "Wenn du das siehst, funktionieren die Benachrichtigungen.".into()),
            },
        };
        NotifyText { title, body }
    }
}

/// Aviso a los jugadores (comando `say`) antes de un reinicio programado.
pub fn restart_warning(lang: Lang, seconds: i64) -> String {
    use Lang::*;
    let (n, unit) = if seconds >= 60 {
        let m = seconds / 60;
        let unit = match (lang, m == 1) {
            (Es | Pt, true) => "minuto",
            (Es | Pt, false) => "minutos",
            (En, true) => "minute",
            (En, false) => "minutes",
            (Fr, true) => "minute",
            (Fr, false) => "minutes",
            (De, true) => "Minute",
            (De, false) => "Minuten",
        };
        (m, unit)
    } else {
        let unit = match (lang, seconds == 1) {
            (Es | Pt, true) => "segundo",
            (Es | Pt, false) => "segundos",
            (En, true) => "second",
            (En, false) => "seconds",
            (Fr, true) => "seconde",
            (Fr, false) => "secondes",
            (De, true) => "Sekunde",
            (De, false) => "Sekunden",
        };
        (seconds, unit)
    };
    match lang {
        Es => format!("El servidor se reiniciará en {n} {unit}."),
        En => format!("The server will restart in {n} {unit}."),
        Pt => format!("O servidor será reiniciado em {n} {unit}."),
        Fr => format!("Le serveur va redémarrer dans {n} {unit}."),
        De => format!("Der Server startet in {n} {unit} neu."),
    }
}

/// Solo webhooks oficiales de Discord y por HTTPS.
pub fn validate_webhook(url: &str) -> Result<()> {
    let invalid = || {
        Error::InvalidConfig(tr!(
            "eso no parece un webhook de Discord (debe empezar por https://discord.com/api/webhooks/)",
            "that does not look like a Discord webhook (it must start with https://discord.com/api/webhooks/)"
        ))
    };
    let parsed = reqwest::Url::parse(url.trim()).map_err(|_| invalid())?;
    let host_ok = matches!(
        parsed.host_str(),
        Some("discord.com" | "discordapp.com" | "ptb.discord.com" | "canary.discord.com")
    );
    if parsed.scheme() != "https"
        || !host_ok
        || !parsed.path().starts_with("/api/webhooks/")
        || parsed.port().is_some()
        || !parsed.username().is_empty()
    {
        return Err(invalid());
    }
    Ok(())
}

/// Envía avisos a un webhook de Discord sin pasarse de su límite (30 por
/// minuto): por encima de 20 por minuto se descartan.
pub struct DiscordSender {
    http: reqwest::Client,
    recent: Mutex<VecDeque<Instant>>,
}

const MAX_PER_MINUTE: usize = 20;

impl DiscordSender {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            recent: Mutex::new(VecDeque::new()),
        }
    }

    fn allow(&self) -> bool {
        let mut recent = self.recent.lock().unwrap();
        let now = Instant::now();
        while recent
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
        {
            recent.pop_front();
        }
        if recent.len() >= MAX_PER_MINUTE {
            return false;
        }
        recent.push_back(now);
        true
    }

    pub async fn send(&self, webhook: &str, text: &NotifyText, tone: Tone) -> Result<()> {
        validate_webhook(webhook)?;
        if !self.allow() {
            tracing::warn!("demasiados avisos a Discord; se descarta uno");
            return Ok(());
        }
        let body = serde_json::json!({
            "username": "Micra",
            // Nadie puede colar un @everyone con el nombre de un servidor.
            "allowed_mentions": { "parse": [] },
            "embeds": [{
                "title": truncate(&text.title, 256),
                "description": truncate(&text.body, 2048),
                "color": tone.color(),
            }],
        });
        self.http
            .post(webhook.trim())
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_has_text_in_every_language() {
        let events = [
            NotifyEvent::PlayerJoined {
                server: "S".into(),
                player: "Steve".into(),
            },
            NotifyEvent::PlayerLeft {
                server: "S".into(),
                player: "Steve".into(),
            },
            NotifyEvent::ServerReady { server: "S".into() },
            NotifyEvent::ServerCrashed {
                server: "S".into(),
                mods: String::new(),
            },
            NotifyEvent::ServerCrashed {
                server: "S".into(),
                mods: "Oculus, Zoomify".into(),
            },
            NotifyEvent::IpBlocked {
                server: "S".into(),
                ip: "203.0.113.1".into(),
                attempts: 8,
            },
            NotifyEvent::PlayerKicked {
                server: "S".into(),
                player: "Bot".into(),
                ip: "203.0.113.1".into(),
            },
            NotifyEvent::Malware {
                server: "S".into(),
                files: "mods/x.jar".into(),
            },
            NotifyEvent::BackupFailed {
                server: "S".into(),
                error: "disco lleno".into(),
            },
            NotifyEvent::Test,
        ];
        for event in &events {
            for lang in Lang::ALL {
                let text = event.text(lang);
                assert!(!text.title.is_empty() && !text.body.is_empty());
            }
        }
        assert_eq!(events[0].text(Lang::Es).title, "Steve ha entrado");
        assert_eq!(events[0].text(Lang::De).title, "Steve ist beigetreten");
    }

    #[test]
    fn settings_control_each_event() {
        let mut s = NotificationSettings::default();
        let join = NotifyEvent::PlayerJoined {
            server: "S".into(),
            player: "A".into(),
        };
        assert!(join.enabled_in(&s));
        s.player_join = false;
        assert!(!join.enabled_in(&s));
        assert!(!NotifyEvent::PlayerLeft {
            server: "S".into(),
            player: "A".into()
        }
        .enabled_in(&s));
    }

    #[test]
    fn restart_warnings() {
        assert_eq!(
            restart_warning(Lang::Es, 600),
            "El servidor se reiniciará en 10 minutos."
        );
        assert_eq!(
            restart_warning(Lang::En, 60),
            "The server will restart in 1 minute."
        );
        assert_eq!(
            restart_warning(Lang::De, 10),
            "Der Server startet in 10 Sekunden neu."
        );
    }

    #[test]
    fn webhooks() {
        assert!(validate_webhook("https://discord.com/api/webhooks/123/abc").is_ok());
        assert!(validate_webhook("https://canary.discord.com/api/webhooks/1/x").is_ok());
        for bad in [
            "http://discord.com/api/webhooks/1/x",
            "https://discord.com.evil.net/api/webhooks/1/x",
            "https://evil.net/api/webhooks/1/x",
            "https://discord.com/api/users/@me",
            "https://user@discord.com/api/webhooks/1/x",
            "https://discord.com:8443/api/webhooks/1/x",
            "nada",
        ] {
            assert!(validate_webhook(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn rate_limit() {
        let sender = DiscordSender::new(reqwest::Client::new());
        for _ in 0..MAX_PER_MINUTE {
            assert!(sender.allow());
        }
        assert!(!sender.allow());
    }
}
