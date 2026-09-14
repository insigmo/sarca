//! Optional append-only client log file for debugging auto-upload on device.
//!
//! Two independent switches, because they answer different questions. `enabled`
//! is "write a log file at all"; the level is "how much of it". The level is
//! what the Settings -> Logs tab calls *advanced logging*: `Info` is the record
//! of what the app did, `Debug` adds the detail you only want while reproducing
//! a problem — every scan decision, every IPC call, every upload verdict.
//!
//! The level also drives the `tracing` subscriber installed in [`install`], so
//! turning advanced logging on reaches the sync engine's own instrumentation
//! and not just the lines written from this crate.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        OnceLock,
    },
};

static ENABLED: AtomicBool = AtomicBool::new(false);
static LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

/// How much detail the log file carries.
///
/// Deliberately only two: a level picker with five entries invites a support
/// conversation about which one to choose, and the only distinction that
/// actually changes what gets diagnosed is "normal" versus "everything".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Info = 0,
    Debug = 1,
}

impl LogLevel {
    /// Parses the wire value. Anything unrecognised is [`LogLevel::Info`]: the
    /// level arrives from prefs on disk and from the WebView, and neither is a
    /// reason to start writing debug logs nobody asked for.
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "debug" | "trace" | "verbose" => Self::Debug,
            _ => Self::Info,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Debug => "debug",
        }
    }

    fn from_u8(raw: u8) -> Self {
        if raw == Self::Debug as u8 {
            Self::Debug
        } else {
            Self::Info
        }
    }
}

pub fn log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("logs").join("sarca-client.log")
}

pub fn set_enabled(enabled: bool, data_dir: &Path) {
    ENABLED.store(enabled, Ordering::Relaxed);
    if enabled {
        let _ = fs::create_dir_all(data_dir.join("logs"));
        write_line(
            data_dir,
            &format!("logging enabled (level={})", level().as_str()),
        );
    }
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn level() -> LogLevel {
    LogLevel::from_u8(LEVEL.load(Ordering::Relaxed))
}

/// True when advanced logging is on — the checkbox in Settings -> Logs.
pub fn is_debug() -> bool {
    level() >= LogLevel::Debug
}

/// Switches the level for the log file *and* for the `tracing` subscriber.
///
/// Takes effect immediately and for already-running work: the subscriber's
/// filter is behind a reload handle, so a sync engine mid-tick starts emitting
/// its `debug!` lines without a restart. That matters — the problems worth
/// turning this on for are the ones already happening.
pub fn set_level(level: LogLevel, data_dir: &Path) {
    let previous = LEVEL.swap(level as u8, Ordering::Relaxed);
    apply_tracing_level(level);
    if LogLevel::from_u8(previous) != level {
        write_line(data_dir, &format!("log level set to {}", level.as_str()));
    }
}

pub fn write_line(data_dir: &Path, msg: &str) {
    append(data_dir, "INFO", msg);
}

/// Detail that is only worth the disk space while reproducing something.
///
/// Cheap to leave in the hot paths: the level check happens before anything
/// touches the filesystem, so a `debug_line` on a per-file scan costs one
/// atomic load while advanced logging is off.
pub fn debug_line(data_dir: &Path, msg: &str) {
    if !is_debug() {
        return;
    }
    append(data_dir, "DEBUG", msg);
}

fn append(data_dir: &Path, level: &str, msg: &str) {
    if !is_enabled() {
        return;
    }
    let path = log_path(data_dir);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = writeln!(file, "{ts} {level} {msg}");
}

/// Read log file contents (capped) for export / share.
pub fn read_export(data_dir: &Path, max_bytes: usize) -> Result<String, String> {
    let path = log_path(data_dir);
    if !path.is_file() {
        return Ok(String::from(
            "(no log file yet — enable logging and reproduce the issue)\n",
        ));
    }
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    if bytes.len() <= max_bytes {
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    }
    let start = bytes.len() - max_bytes;
    Ok(format!(
        "(truncated, showing last {max_bytes} bytes)\n{}",
        String::from_utf8_lossy(&bytes[start..])
    ))
}

/// Remove the log file. Returns the bytes reclaimed.
pub fn clear(data_dir: &Path) -> Result<u64, String> {
    let path = log_path(data_dir);
    let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    match fs::remove_file(&path) {
        Ok(()) => Ok(size),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e.to_string()),
    }
}

/// Size of the log file on disk, or 0 when there is none.
pub fn size_bytes(data_dir: &Path) -> u64 {
    fs::metadata(log_path(data_dir)).map(|m| m.len()).unwrap_or(0)
}

//////////////////////////////////////
//      tracing bridge
//////////////////////////////////////

type ReloadHandle = tracing_subscriber::reload::Handle<
    tracing_subscriber::filter::LevelFilter,
    tracing_subscriber::Registry,
>;

static RELOAD: OnceLock<ReloadHandle> = OnceLock::new();

/// Sink that puts `tracing` events into the same file as [`write_line`].
///
/// The engine, the API client and the scheduler are all instrumented with
/// `tracing` already, and until this existed none of it was written anywhere on
/// desktop — the log file held only what this crate chose to record by hand.
/// The interesting half of an auto-upload failure lives in the other crate.
struct ClientLogWriter {
    data_dir: PathBuf,
}

impl Write for ClientLogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let len = buf.len();
        if is_enabled() {
            let path = log_path(&self.data_dir);
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
                let _ = file.write_all(buf);
            }
        }
        Ok(len)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Installs the `tracing` subscriber that writes into the client log file.
///
/// Safe to call more than once — only the first call wins, and a later one is a
/// no-op rather than a panic, because the app's entry point runs twice on some
/// mobile launch paths.
pub fn install(data_dir: PathBuf) {
    use tracing_subscriber::{fmt, layer::SubscriberExt as _, reload, util::SubscriberInitExt as _};

    if RELOAD.get().is_some() {
        return;
    }
    let (filter, handle) = reload::Layer::new(level_filter(level()));
    let dir = data_dir;
    let layer = fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_writer(move || ClientLogWriter {
            data_dir: dir.clone(),
        });
    if tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .try_init()
        .is_ok()
    {
        let _ = RELOAD.set(handle);
    }
}

fn level_filter(level: LogLevel) -> tracing_subscriber::filter::LevelFilter {
    match level {
        LogLevel::Info => tracing_subscriber::filter::LevelFilter::INFO,
        LogLevel::Debug => tracing_subscriber::filter::LevelFilter::DEBUG,
    }
}

fn apply_tracing_level(level: LogLevel) {
    if let Some(handle) = RELOAD.get() {
        let _ = handle.modify(|f| *f = level_filter(level));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    /// `ENABLED` and `LEVEL` are process-global, which is the point — one
    /// switch for the whole app — but it means two of these tests running at
    /// once would flip each other's state. Each takes this first.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("sarca-log-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_and_read_when_enabled() {
        let _serial = lock();
        let dir = scratch("basic");
        set_enabled(true, &dir);
        write_line(&dir, "hello-auto-upload");
        let text = read_export(&dir, 64 * 1024).unwrap();
        assert!(text.contains("hello-auto-upload"));
        set_enabled(false, &dir);
        let _ = fs::remove_dir_all(&dir);
    }

    /// The whole point of the advanced-logging checkbox: at the default level a
    /// debug line costs nothing and says nothing.
    #[test]
    fn debug_lines_are_only_written_at_the_debug_level() {
        let _serial = lock();
        let dir = scratch("level");
        set_enabled(true, &dir);
        set_level(LogLevel::Info, &dir);
        debug_line(&dir, "quiet-detail");
        assert!(!read_export(&dir, 64 * 1024)
            .unwrap()
            .contains("quiet-detail"));

        set_level(LogLevel::Debug, &dir);
        debug_line(&dir, "loud-detail");
        assert!(read_export(&dir, 64 * 1024).unwrap().contains("loud-detail"));

        set_level(LogLevel::Info, &dir);
        set_enabled(false, &dir);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Prefs on disk and the WebView both feed this, so an unknown value must
    /// land on the quiet side rather than the verbose one.
    #[test]
    fn an_unknown_level_reads_as_info() {
        assert_eq!(LogLevel::parse("debug"), LogLevel::Debug);
        assert_eq!(LogLevel::parse("DEBUG"), LogLevel::Debug);
        assert_eq!(LogLevel::parse("info"), LogLevel::Info);
        assert_eq!(LogLevel::parse("shout"), LogLevel::Info);
        assert_eq!(LogLevel::parse(""), LogLevel::Info);
    }

    #[test]
    fn clearing_removes_the_file_and_reports_the_bytes() {
        let _serial = lock();
        let dir = scratch("clear");
        set_enabled(true, &dir);
        write_line(&dir, "something to reclaim");
        assert!(size_bytes(&dir) > 0);
        assert!(clear(&dir).unwrap() > 0);
        assert_eq!(size_bytes(&dir), 0);
        // Clearing a log that is not there is not an error.
        assert_eq!(clear(&dir).unwrap(), 0);
        set_enabled(false, &dir);
        let _ = fs::remove_dir_all(&dir);
    }
}
