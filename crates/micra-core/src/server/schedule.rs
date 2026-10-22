//! Reinicios programados y apagado por inactividad.
//!
//! Es lógica pura (sin E/S ni temporizadores): quien la usa llama a
//! [`Scheduler::tick`] cada pocos segundos con la hora actual y el número de
//! jugadores, y ejecuta la acción que devuelve. Así se puede probar con horas
//! inventadas, incluidos los cambios de horario de verano.

use chrono::{DateTime, Duration, NaiveTime, TimeZone};

use crate::server::config::SchedulePolicy;

/// Avisos antes de un reinicio, en segundos.
pub const WARNINGS: [i64; 4] = [600, 300, 60, 10];

/// Un reinicio diario nunca ocurre antes de este tiempo encendido: si el
/// servidor arranca a las 03:58 y el reinicio es a las 04:00, se pasa al día
/// siguiente.
const MIN_UPTIME_BEFORE_DAILY: i64 = 30 * 60;

pub fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    let (h, m) = s.trim().split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    NaiveTime::from_hms_opt(h.parse().ok()?, m.parse().ok()?, 0)
}

/// Próximo instante, estrictamente posterior a `after`, en el que el reloj
/// local marca `time`. Si esa hora no existe ese día (adelanto de hora), se
/// usa la primera hora válida después.
pub fn next_daily<Tz: TimeZone>(after: &DateTime<Tz>, time: NaiveTime) -> DateTime<Tz> {
    let tz = after.timezone();
    let mut date = after.date_naive();
    for _ in 0..4 {
        let naive = date.and_time(time);
        let candidate = tz.from_local_datetime(&naive).earliest().or_else(|| {
            tz.from_local_datetime(&(naive + Duration::hours(1)))
                .earliest()
        });
        if let Some(candidate) = candidate {
            if candidate > *after {
                return candidate;
            }
        }
        date = date.succ_opt().unwrap_or(date);
    }
    after.clone() + Duration::days(1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleAction {
    /// Avisar a los jugadores de que quedan `seconds` para el reinicio.
    Warn {
        seconds: i64,
    },
    Restart,
    /// Nadie conectado durante el tiempo configurado.
    IdleStop,
}

#[derive(Debug, Clone)]
pub struct Scheduler<Tz: TimeZone> {
    restart_at: Option<DateTime<Tz>>,
    warned: Vec<i64>,
    idle_limit: Option<Duration>,
    empty_since: Option<DateTime<Tz>>,
    idle_fired: bool,
}

impl<Tz: TimeZone> Scheduler<Tz> {
    /// `started_at`: cuándo arrancó (o pasó a "en línea") el servidor.
    pub fn new(policy: &SchedulePolicy, started_at: DateTime<Tz>) -> Self {
        let daily = policy
            .daily_restart
            .as_deref()
            .and_then(parse_hhmm)
            .map(|time| {
                let earliest = started_at.clone() + Duration::seconds(MIN_UPTIME_BEFORE_DAILY);
                let first = next_daily(&started_at, time);
                if first < earliest {
                    next_daily(&first, time)
                } else {
                    first
                }
            });
        let every = policy
            .restart_every_hours
            .filter(|h| *h > 0)
            .map(|h| started_at.clone() + Duration::hours(i64::from(h)));
        let restart_at = match (daily, every) {
            (Some(a), Some(b)) => Some(if a < b { a } else { b }),
            (a, b) => a.or(b),
        };
        Self {
            restart_at,
            warned: Vec::new(),
            idle_limit: policy
                .idle_shutdown_minutes
                .filter(|m| *m > 0)
                .map(|m| Duration::minutes(i64::from(m))),
            empty_since: Some(started_at),
            idle_fired: false,
        }
    }

    pub fn restart_at(&self) -> Option<&DateTime<Tz>> {
        self.restart_at.as_ref()
    }

    pub fn tick(&mut self, now: DateTime<Tz>, players: usize) -> Option<ScheduleAction> {
        if let Some(at) = self.restart_at.clone() {
            let left = (at - now.clone()).num_seconds();
            if left <= 0 {
                self.restart_at = None;
                return Some(ScheduleAction::Restart);
            }
            // El primer aviso que toca y que no se ha dado. Si el reloj saltó
            // (portátil suspendido), se avisa solo con el más cercano.
            let due: Vec<i64> = WARNINGS
                .iter()
                .copied()
                .filter(|w| left <= *w && !self.warned.contains(w))
                .collect();
            if let Some(&closest) = due.iter().min() {
                self.warned.extend(due);
                if players > 0 {
                    return Some(ScheduleAction::Warn { seconds: closest });
                }
            }
        }

        if let Some(limit) = self.idle_limit {
            if players > 0 {
                self.empty_since = None;
                self.idle_fired = false;
            } else {
                let since = self.empty_since.get_or_insert_with(|| now.clone()).clone();
                if !self.idle_fired && now - since >= limit {
                    self.idle_fired = true;
                    return Some(ScheduleAction::IdleStop);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, Utc};

    fn at(s: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(s).unwrap()
    }

    fn policy(daily: Option<&str>, every: Option<u32>, idle: Option<u32>) -> SchedulePolicy {
        SchedulePolicy {
            daily_restart: daily.map(str::to_string),
            restart_every_hours: every,
            idle_shutdown_minutes: idle,
        }
    }

    #[test]
    fn parses_times() {
        assert_eq!(
            parse_hhmm("4:05"),
            Some(NaiveTime::from_hms_opt(4, 5, 0).unwrap())
        );
        assert_eq!(
            parse_hhmm(" 23:59 "),
            Some(NaiveTime::from_hms_opt(23, 59, 0).unwrap())
        );
        for bad in ["24:00", "12:60", "12", "1:5", "aa:bb", ":30"] {
            assert_eq!(parse_hhmm(bad), None, "{bad}");
        }
    }

    #[test]
    fn daily_restart_skips_to_tomorrow_when_too_close() {
        let p = policy(Some("04:00"), None, None);
        let s = Scheduler::new(&p, at("2026-10-22T03:58:00+02:00"));
        assert_eq!(s.restart_at(), Some(&at("2026-10-23T04:00:00+02:00")));
        let s = Scheduler::new(&p, at("2026-10-22T01:00:00+02:00"));
        assert_eq!(s.restart_at(), Some(&at("2026-10-22T04:00:00+02:00")));
    }

    #[test]
    fn earliest_of_daily_and_interval() {
        let p = policy(Some("04:00"), Some(2), None);
        let s = Scheduler::new(&p, at("2026-10-22T01:00:00+02:00"));
        assert_eq!(s.restart_at(), Some(&at("2026-10-22T03:00:00+02:00")));
    }

    #[test]
    fn warns_then_restarts() {
        let p = policy(None, Some(1), None);
        let start = at("2026-10-22T10:00:00+00:00");
        let mut s = Scheduler::new(&p, start);
        assert_eq!(s.tick(at("2026-10-22T10:40:00+00:00"), 2), None);
        assert_eq!(
            s.tick(at("2026-10-22T10:50:00+00:00"), 2),
            Some(ScheduleAction::Warn { seconds: 600 })
        );
        assert_eq!(s.tick(at("2026-10-22T10:51:00+00:00"), 2), None);
        // Salto grande: solo el aviso más cercano.
        assert_eq!(
            s.tick(at("2026-10-22T10:59:30+00:00"), 2),
            Some(ScheduleAction::Warn { seconds: 60 })
        );
        assert_eq!(s.tick(at("2026-10-22T10:59:40+00:00"), 2), None);
        assert_eq!(
            s.tick(at("2026-10-22T10:59:52+00:00"), 2),
            Some(ScheduleAction::Warn { seconds: 10 })
        );
        assert_eq!(
            s.tick(at("2026-10-22T11:00:00+00:00"), 2),
            Some(ScheduleAction::Restart)
        );
        assert_eq!(s.tick(at("2026-10-22T11:00:05+00:00"), 2), None);
    }

    #[test]
    fn no_warnings_without_players() {
        let p = policy(None, Some(1), None);
        let mut s = Scheduler::new(&p, at("2026-10-22T10:00:00+00:00"));
        assert_eq!(s.tick(at("2026-10-22T10:55:00+00:00"), 0), None);
        assert_eq!(
            s.tick(at("2026-10-22T11:00:00+00:00"), 0),
            Some(ScheduleAction::Restart)
        );
    }

    #[test]
    fn idle_shutdown() {
        let p = policy(None, None, Some(15));
        let mut s = Scheduler::new(&p, at("2026-10-22T10:00:00+00:00"));
        assert_eq!(s.tick(at("2026-10-22T10:10:00+00:00"), 0), None);
        // Alguien entra: el contador se reinicia.
        assert_eq!(s.tick(at("2026-10-22T10:12:00+00:00"), 1), None);
        assert_eq!(s.tick(at("2026-10-22T10:20:00+00:00"), 0), None);
        assert_eq!(s.tick(at("2026-10-22T10:34:00+00:00"), 0), None);
        assert_eq!(
            s.tick(at("2026-10-22T10:35:00+00:00"), 0),
            Some(ScheduleAction::IdleStop)
        );
        assert_eq!(s.tick(at("2026-10-22T10:36:00+00:00"), 0), None);
    }

    #[test]
    fn next_daily_handles_utc() {
        let now = Utc.with_ymd_and_hms(2026, 3, 29, 23, 0, 0).unwrap();
        let next = next_daily(&now, NaiveTime::from_hms_opt(4, 0, 0).unwrap());
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 3, 30, 4, 0, 0).unwrap());
    }
}
