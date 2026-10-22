//! Android foreground service that keeps the process — and so the sync loop —
//! alive while auto-upload is on, plus the wake/Wi-Fi locks held while a pass
//! is actually moving files.
//!
//! Without it Android froze the backgrounded process mid-upload, the server
//! saw the stream stall, and the upload died on an idle timeout. The service
//! is `specialUse`, not `dataSync`: Android 15 caps `dataSync` at six hours a
//! day, and the APK is sideloaded, so Play's review of the type does not apply.
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

pub struct BusyGuard(BgService);

impl Drop for BusyGuard {
    fn drop(&mut self) {
        if self.0.busy.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.0.call("setBusy", serde_json::json!({ "busy": false }));
        }
    }
}
