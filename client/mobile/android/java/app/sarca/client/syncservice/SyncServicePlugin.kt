package app.sarca.client.syncservice

import android.app.Activity
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

/**
 * Starts/stops [SyncForegroundService] and tells it when a sync pass is moving
 * files (see client/src-tauri/src/bg_service.rs).
 * Installed into the generated Android project by scripts/patch-android-http.sh.
 */
@TauriPlugin
class SyncServicePlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun start(invoke: Invoke) {
    try {
      SyncForegroundService.start(activity.applicationContext)
      invoke.resolve(JSObject())
    } catch (ex: Exception) {
      // Android 12+ refuses a foreground-service start from the background
      // unless the app is exempt from battery optimizations. The next start
      // while the app is open will succeed.
      invoke.reject(ex.message ?: "sync service start refused")
    }
  }

  @Command
  fun stop(invoke: Invoke) {
    SyncForegroundService.stop()
    invoke.resolve(JSObject())
  }

  @Command
  fun setBusy(invoke: Invoke) {
    SyncForegroundService.setBusy(invoke.getArgs().optBoolean("busy", false))
    invoke.resolve(JSObject())
  }
}
