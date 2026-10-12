use std::io::{self, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use log::kv::{Key, Value, VisitSource};
use log::{LevelFilter, Metadata, Record};

struct EngineLogger;

static ENGINE_LOGGER: EngineLogger = EngineLogger;

impl log::Log for EngineLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let mut fields = FieldCollector::default();
        let _ = record.key_values().visit(&mut fields);
        let source = match (record.file(), record.line()) {
            (Some(file), Some(line)) => format!(" {file}:{line}"),
            _ => String::new(),
        };
        let structured_fields = if fields.0.is_empty() { String::new() } else { format!(" {}", fields.0.join(" ")) };
        let mut stderr = io::stderr().lock();
        let _ = writeln!(stderr, "{} {:<5} {}{} {}{}", timestamp_utc(), record.level(), record.target(), source, record.args(), structured_fields);
    }

    fn flush(&self) {}
}

#[derive(Default)]
struct FieldCollector(Vec<String>);

impl<'kvs> VisitSource<'kvs> for FieldCollector {
    fn visit_pair(&mut self, key: Key<'kvs>, value: Value<'kvs>) -> Result<(), log::kv::Error> {
        self.0.push(format!("{key}={value:?}"));
        Ok(())
    }
}

/// Installs the engine's timestamped structured logger unless the application installed
/// its own logger first. Set `LABYRINTH_LOG` to trace, debug, info, warn, error, or off.
pub fn init_logging() {
    if log::set_logger(&ENGINE_LOGGER).is_ok() {
        log::set_max_level(configured_level());
    }
}

fn configured_level() -> LevelFilter {
    match std::env::var("LABYRINTH_LOG").as_deref().unwrap_or("info") {
        "trace" => LevelFilter::Trace,
        "debug" => LevelFilter::Debug,
        "info" => LevelFilter::Info,
        "warn" => LevelFilter::Warn,
        "error" => LevelFilter::Error,
        "off" => LevelFilter::Off,
        _ => LevelFilter::Info,
    }
}

fn timestamp_utc() -> String {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let unix_seconds = duration.as_secs().min(i64::MAX as u64) as i64;
    let days = unix_seconds.div_euclid(86_400);
    let day_seconds = unix_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_date_from_days(days);
    let hour = day_seconds / 3_600;
    let minute = day_seconds % 3_600 / 60;
    let second = day_seconds % 60;
    let milliseconds = duration.subsec_millis();
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milliseconds:03}Z")
}

fn civil_date_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let shifted_days = days_since_epoch + 719_468;
    let era = (if shifted_days >= 0 { shifted_days } else { shifted_days - 146_096 }).div_euclid(146_097);
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}
