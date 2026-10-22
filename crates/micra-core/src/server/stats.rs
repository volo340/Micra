//! Estadísticas históricas: un punto por minuto (CPU, RAM, jugadores y TPS)
//! en un CSV por día y servidor, `stats/<id>/AAAA-MM-DD.csv` (fecha UTC).
//! Se guardan 31 días.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use serde::Serialize;

use crate::Result;

const HEADER: &str = "ts,cpu,ram_mb,players,tps\n";
pub const KEEP_DAYS: i64 = 31;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatPoint {
    /// Segundos desde 1970 (UTC).
    pub ts: i64,
    pub cpu: f32,
    pub ram_mb: u64,
    pub players: u32,
    pub tps: Option<f32>,
}

/// Junta las muestras de cada segundo en un punto por minuto.
#[derive(Debug, Default)]
pub struct MinuteAccumulator {
    minute: Option<i64>,
    samples: u32,
    cpu: f64,
    ram: u64,
    players: u32,
    tps: Option<f32>,
}

impl MinuteAccumulator {
    /// Añade una muestra; si empieza un minuto nuevo, devuelve el anterior.
    pub fn push(
        &mut self,
        at: DateTime<Utc>,
        cpu: f32,
        ram_mb: u64,
        players: u32,
        tps: Option<f32>,
    ) -> Option<StatPoint> {
        let minute = at.timestamp() / 60;
        let finished = match self.minute {
            Some(m) if m != minute => self.flush(),
            _ => None,
        };
        self.minute = Some(minute);
        self.samples += 1;
        self.cpu += cpu as f64;
        self.ram = self.ram.max(ram_mb);
        self.players = self.players.max(players);
        if tps.is_some() {
            self.tps = tps;
        }
        finished
    }

    /// El punto del minuto en curso (al apagar el servidor).
    pub fn flush(&mut self) -> Option<StatPoint> {
        let minute = self.minute.take()?;
        if self.samples == 0 {
            return None;
        }
        let point = StatPoint {
            ts: minute * 60,
            cpu: (self.cpu / self.samples as f64) as f32,
            ram_mb: self.ram,
            players: self.players,
            tps: self.tps,
        };
        *self = Self::default();
        Some(point)
    }
}

fn day_file(dir: &Path, day: NaiveDate) -> PathBuf {
    dir.join(format!("{}.csv", day.format("%Y-%m-%d")))
}

pub fn append(dir: &Path, point: &StatPoint) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let day = Utc
        .timestamp_opt(point.ts, 0)
        .single()
        .unwrap_or_else(Utc::now)
        .date_naive();
    let path = day_file(dir, day);
    let is_new = !path.exists();
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    if is_new {
        file.write_all(HEADER.as_bytes())?;
    }
    let tps = point.tps.map(|t| format!("{t:.2}")).unwrap_or_default();
    writeln!(
        file,
        "{},{:.1},{},{},{}",
        point.ts, point.cpu, point.ram_mb, point.players, tps
    )?;
    Ok(())
}

fn parse_line(line: &str) -> Option<StatPoint> {
    let mut parts = line.split(',');
    Some(StatPoint {
        ts: parts.next()?.parse().ok()?,
        cpu: parts.next()?.parse().ok()?,
        ram_mb: parts.next()?.parse().ok()?,
        players: parts.next()?.parse().ok()?,
        tps: parts.next().and_then(|t| t.parse().ok()),
    })
}

/// Puntos entre `from` y `to`, reducidos a como mucho `max_points`
/// (media de CPU, RAM y TPS; máximo de jugadores en cada tramo).
pub fn read_range(
    dir: &Path,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    max_points: usize,
) -> Vec<StatPoint> {
    let mut points = Vec::new();
    let mut day = from.date_naive();
    while day <= to.date_naive() {
        if let Ok(content) = std::fs::read_to_string(day_file(dir, day)) {
            points.extend(
                content
                    .lines()
                    .skip(1)
                    .filter_map(parse_line)
                    .filter(|p| p.ts >= from.timestamp() && p.ts <= to.timestamp()),
            );
        }
        day += Duration::days(1);
    }
    points.sort_by_key(|p| p.ts);
    downsample(&points, from.timestamp(), to.timestamp(), max_points)
}

fn downsample(points: &[StatPoint], from: i64, to: i64, max_points: usize) -> Vec<StatPoint> {
    if points.len() <= max_points || max_points == 0 {
        return points.to_vec();
    }
    let span = (to - from).max(1) as f64;
    let bucket_of = |ts: i64| (((ts - from) as f64 / span) * max_points as f64) as usize;
    let mut out: Vec<StatPoint> = Vec::with_capacity(max_points);
    let mut i = 0;
    while i < points.len() {
        let bucket = bucket_of(points[i].ts).min(max_points - 1);
        let mut j = i;
        while j < points.len() && bucket_of(points[j].ts).min(max_points - 1) == bucket {
            j += 1;
        }
        let group = &points[i..j];
        let n = group.len() as f32;
        let tps: Vec<f32> = group.iter().filter_map(|p| p.tps).collect();
        out.push(StatPoint {
            ts: group[group.len() / 2].ts,
            cpu: group.iter().map(|p| p.cpu).sum::<f32>() / n,
            ram_mb: (group.iter().map(|p| p.ram_mb as f64).sum::<f64>() / n as f64) as u64,
            players: group.iter().map(|p| p.players).max().unwrap_or(0),
            tps: (!tps.is_empty()).then(|| tps.iter().sum::<f32>() / tps.len() as f32),
        });
        i = j;
    }
    out
}

/// Borra los días más antiguos que `KEEP_DAYS`.
pub fn prune(dir: &Path, now: DateTime<Utc>) -> Result<usize> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(0);
    };
    let limit = (now - Duration::days(KEEP_DAYS)).date_naive();
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(day) = name
            .strip_suffix(".csv")
            .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        else {
            continue;
        };
        if day < limit {
            std::fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn accumulates_per_minute() {
        let mut acc = MinuteAccumulator::default();
        assert!(acc
            .push(at("2026-10-22T10:00:01Z"), 10.0, 1000, 1, None)
            .is_none());
        assert!(acc
            .push(at("2026-10-22T10:00:30Z"), 30.0, 1500, 3, Some(19.5))
            .is_none());
        let p = acc
            .push(at("2026-10-22T10:01:00Z"), 50.0, 900, 0, None)
            .unwrap();
        assert_eq!(p.ts, at("2026-10-22T10:00:00Z").timestamp());
        assert_eq!(p.cpu, 20.0);
        assert_eq!(p.ram_mb, 1500);
        assert_eq!(p.players, 3);
        assert_eq!(p.tps, Some(19.5));
        let last = acc.flush().unwrap();
        assert_eq!(last.cpu, 50.0);
        assert!(acc.flush().is_none());
    }

    #[test]
    fn writes_reads_downsamples_and_prunes() {
        let dir = tempfile::tempdir().unwrap();
        let start = at("2026-10-21T23:30:00Z");
        for i in 0..60 {
            append(
                dir.path(),
                &StatPoint {
                    ts: start.timestamp() + i * 60,
                    cpu: i as f32,
                    ram_mb: 1000 + i as u64,
                    players: (i % 5) as u32,
                    tps: if i % 2 == 0 { Some(20.0) } else { None },
                },
            )
            .unwrap();
        }
        // Medianoche UTC: dos archivos.
        assert!(dir.path().join("2026-10-21.csv").exists());
        assert!(dir.path().join("2026-10-22.csv").exists());

        let all = read_range(dir.path(), start, start + Duration::hours(1), 1000);
        assert_eq!(all.len(), 60);
        assert_eq!(all[0].tps, Some(20.0));
        assert_eq!(all[1].tps, None);

        let few = read_range(dir.path(), start, start + Duration::hours(1), 6);
        assert!(few.len() <= 6);
        assert_eq!(few[0].players, 4);

        let removed = prune(dir.path(), at("2026-11-22T00:00:00Z")).unwrap();
        assert_eq!(removed, 1);
        assert!(dir.path().join("2026-10-22.csv").exists());
    }
}
