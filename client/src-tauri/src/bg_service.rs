//! Android foreground service that keeps the process — and so the sync loop —
//! alive while auto-upload is on, plus the wake/Wi-Fi locks held while a pass
//! is actually moving files.
//!
//! Without it Android froze the backgrounded process mid-upload, the server
//! saw the stream stall, and the upload died on an idle timeout. The service
//! is `specialUse`, not `dataSync`: Android 15 caps `dataSync` at six hours a
//! day, and the APK is sideloaded, so Play's review of the type does not apply.
//!
//! The service also wakes the loop. Between passes nothing holds the CPU up,
//! and `tokio::time::sleep` runs on a clock that stops while the device is
//! suspended, so a 15-minute wait could stretch to hours and auto-upload looked
//! like it had stopped. The service sets an alarm that fires through Doze and a
//! watcher on the gallery, and either one reaches the loop through
//! [`BgService::attach`].
//!
//! While a pass runs, the service's notification shows how many files are done
//! and left, and a second one shows the file on the wire.
//!
//! Known limit: the service is started from Rust, and Rust only runs once the
//! app has been opened, so after a reboot or a process kill auto-upload resumes
//! when the user next opens Sarca.
//!
//! Every call is a no-op on other platforms.

use std::sync::{
    atomic::{AtomicU8, AtomicUsize, Ordering},
    Arc,
};

use sarca_sync::{engine::SyncEngine, UploadProgressSnapshot};
use serde::Serialize;
use tauri::{
    plugin::{Builder as PluginBuilder, TauriPlugin},
    AppHandle, Runtime,
};

/// Register the Android sync-service plugin (no-op setup on other targets).
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    PluginBuilder::<R, ()>::new("sarca-sync-service")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle = api
                    .register_android_plugin("app.sarca.client.syncservice", "SyncServicePlugin")?;
                app.manage(AndroidSyncService { handle });
            }
            #[cfg(not(target_os = "android"))]
            {
                let _ = (app, api);
            }
            Ok(())
        })
        .build()
}

#[cfg(target_os = "android")]
struct AndroidSyncService<R: Runtime> {
    handle: tauri::plugin::PluginHandle<R>,
}

const UNKNOWN: u8 = 0;
const STOPPED: u8 = 1;
const RUNNING: u8 = 2;

/// Handle the sync loop and `sync_now` use to drive the service. Cheap to
/// clone; clones share state.
#[derive(Clone)]
pub struct BgService {
    /// `None` for [`BgService::inert`]. Android-only: owning an `AppHandle`
    /// anywhere the unit tests reach links Tauri's runtime into the test
    /// binary, which on Windows then fails to start (`STATUS_ENTRYPOINT_NOT_FOUND`).
    #[cfg(target_os = "android")]
    app: Option<AppHandle>,
    /// Last state asked of the service, so a tick that changes nothing sends
    /// nothing across the bridge.
    wanted: Arc<AtomicU8>,
    /// Passes in flight. The locks are held while this is above zero, so an
    /// overlapping manual sync and loop tick do not release them early.
    busy: Arc<AtomicUsize>,
}

impl BgService {
    pub fn new(app: &AppHandle) -> Self {
        #[cfg(not(target_os = "android"))]
        let _ = app;
        Self {
            #[cfg(target_os = "android")]
            app: Some(app.clone()),
            wanted: Arc::new(AtomicU8::new(UNKNOWN)),
            busy: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// A service handle that drives nothing, for state built without an app.
    #[cfg(test)]
    pub fn inert() -> Self {
        Self {
            #[cfg(target_os = "android")]
            app: None,
            wanted: Arc::new(AtomicU8::new(UNKNOWN)),
            busy: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Runs the service while `on` (auto-upload enabled and signed in), stops
    /// it otherwise.
    pub fn set_running(&self, on: bool) {
        let next = if on { RUNNING } else { STOPPED };
        if self.wanted.swap(next, Ordering::SeqCst) != next {
            self.call(if on { "start" } else { "stop" }, serde_json::json!({}));
        }
    }

    /// Feeds the notification with upload progress and turns the service's
    /// alarm and gallery signals into loop wake-ups (a bump of `wake_tx`).
    /// Called once per process, by `start_background_loop`.
    pub fn attach(&self, engine: Arc<SyncEngine>, wake_tx: tokio::sync::watch::Sender<u64>) {
        #[cfg(target_os = "android")]
        {
            use tauri::Manager;
            let Some(state) = self
                .app
                .as_ref()
                .and_then(|app| app.try_state::<AndroidSyncService<tauri::Wry>>())
            else {
                return;
            };
            let handle = state.handle.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    // Parked in the service until its alarm fires or the gallery
                    // changes. The async variant keeps no thread blocked on it.
                    match handle
                        .run_mobile_plugin_async::<serde_json::Value>(
                            "awaitWake",
                            serde_json::json!({}),
                        )
                        .await
                    {
                        Ok(_) => {
                            let next = wake_tx.borrow().wrapping_add(1);
                            let _ = wake_tx.send(next);
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "sync service wake wait failed");
                            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                        }
                    }
                }
            });
            let bg = self.clone();
            tauri::async_runtime::spawn(async move { bg.feed_progress(engine).await });
        }
        #[cfg(not(target_os = "android"))]
        let _ = (engine, wake_tx);
    }

    /// Sends the notification what [`progress_note`] makes of the queue, once
    /// a second while a pass runs and only when it changed.
    #[cfg(target_os = "android")]
    async fn feed_progress(&self, engine: Arc<SyncEngine>) {
        let mut base = engine.upload_progress().await;
        let mut last: Option<ProgressNote> = None;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if self.busy.load(Ordering::SeqCst) == 0 {
                if last.take().is_some() {
                    // The pass is over; `setBusy(false)` has already reset the
                    // notification. A drained queue starts the count afresh.
                    let now = engine.upload_progress().await;
                    if now.remaining == 0 {
                        base = now;
                    }
                }
                continue;
            }
            let now = engine.upload_progress().await;
            if now.remaining == 0 && now.current.is_empty() {
                base = now.clone();
            }
            let note = progress_note(&base, &now);
            if last.as_ref() != Some(&note) {
                self.call(
                    "setProgress",
                    serde_json::to_value(&note).unwrap_or_default(),
                );
                last = Some(note);
            }
        }
    }

    /// Marks a pass as running until the returned guard drops.
    pub fn busy(&self) -> BusyGuard {
        if self.busy.fetch_add(1, Ordering::SeqCst) == 0 {
            self.call("setBusy", serde_json::json!({ "busy": true }));
        }
        BusyGuard(self.clone())
    }

    #[cfg(target_os = "android")]
    fn call(&self, command: &'static str, args: serde_json::Value) {
        use tauri::Manager;
        let Some(state) = self
            .app
            .as_ref()
            .and_then(|app| app.try_state::<AndroidSyncService<tauri::Wry>>())
        else {
            return;
        };
        let handle = state.handle.clone();
        let wanted = self.wanted.clone();
        // `run_mobile_plugin` blocks on the main looper; never from a runtime worker.
        std::thread::spawn(move || {
            if let Err(e) = handle.run_mobile_plugin::<serde_json::Value>(command, args) {
                tracing::warn!(command, error = %e, "sync service call failed");
                // A refused start (Android blocks most starts from the
                // background) must be retried by the next tick, not remembered
                // as done.
                if command == "start" {
                    let _ = wanted.compare_exchange(
                        RUNNING,
                        UNKNOWN,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    );
                }
            }
        });
    }

    #[cfg(not(target_os = "android"))]
    fn call(&self, command: &'static str, args: serde_json::Value) {
        let _ = (command, args);
    }
}

/// What the notifications show: the run's file counts and byte percentage,
/// and the file on the wire, if any.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressNote {
    pub done: u64,
    pub remaining: usize,
    pub total: u64,
    pub percent: u8,
    pub file: Option<String>,
    pub file_percent: u8,
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn percent_of(part: u64, whole: u64) -> u8 {
    if whole == 0 {
        return 0;
    }
    (part.min(whole) as u128 * 100 / whole as u128) as u8
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
/// Counts since `base`, the queue as it was when this run began. The
/// percentage is by bytes, so one large video weighs what it costs to send.
pub fn progress_note(base: &UploadProgressSnapshot, now: &UploadProgressSnapshot) -> ProgressNote {
    let done = now.completed.saturating_sub(base.completed);
    let done_bytes = now.completed_bytes.saturating_sub(base.completed_bytes);
    let sent: u64 = now.current.iter().map(|f| f.sent.min(f.size)).sum();
    let file = now.current.first();
    ProgressNote {
        done,
        remaining: now.remaining,
        total: done + now.remaining as u64,
        percent: percent_of(done_bytes + sent, done_bytes + now.remaining_bytes),
        file: file.map(|f| f.name.clone()),
        file_percent: file.map(|f| percent_of(f.sent, f.size)).unwrap_or(0),
    }
}

pub struct BusyGuard(BgService);

impl Drop for BusyGuard {
    fn drop(&mut self) {
        if self.0.busy.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.0.call("setBusy", serde_json::json!({ "busy": false }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sarca_sync::UploadInFlight;

    fn snap(
        completed: u64,
        completed_bytes: u64,
        remaining: usize,
        remaining_bytes: u64,
    ) -> UploadProgressSnapshot {
        UploadProgressSnapshot {
            completed,
            completed_bytes,
            remaining,
            remaining_bytes,
            current: Vec::new(),
        }
    }

    #[test]
    fn note_counts_from_the_start_of_the_run_and_weighs_by_bytes() {
        let base = snap(5, 500, 4, 1000);
        let mut now = snap(7, 800, 2, 700);
        now.current.push(UploadInFlight {
            name: "VID_1.mp4".into(),
            sent: 150,
            size: 600,
        });
        let note = progress_note(&base, &now);
        assert_eq!((note.done, note.remaining, note.total), (2, 2, 4));
        // 300 finished + 150 on the wire, of 300 + 700.
        assert_eq!(note.percent, 45);
        assert_eq!(note.file.as_deref(), Some("VID_1.mp4"));
        assert_eq!(note.file_percent, 25);
    }

    #[test]
    fn empty_run_is_zero_not_a_division_by_zero() {
        let note = progress_note(&snap(0, 0, 0, 0), &snap(0, 0, 0, 0));
        assert_eq!((note.total, note.percent, note.file_percent), (0, 0, 0));
        assert_eq!(note.file, None);
    }
}
