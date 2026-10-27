package app.sarca.client

import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import app.sarca.client.syncservice.SyncForegroundService

/**
 * Tauri's generated activity (same `onCreate`), plus one rule: closing the app —
 * swiping it out of Recents — must not end auto-upload.
 *
 * tao reports the destruction of an activity as its window being destroyed, and
 * Tauri answers the last window going away with `exit(0)`: the whole process
 * ends, the foreground service and the upload in flight with it, and nothing
 * runs again until the app is opened. Every "silent" stop in the client logs of
 * 2026-10-26/27 was this — `EXIT_SELF`, status 0, at foreground-service
 * importance, seconds after the user opened Recents.
 *
 * Telling Rust to stay alive is not enough on its own: every Rust → Kotlin
 * plugin call (MediaStore listing, the service's wake/progress calls) is
 * dispatched through a registered activity, and wry unregisters it on a real
 * destroy. So while the sync service runs, destruction is reported as a
 * configuration change instead. tao and wry then keep the Rust window, the
 * webview state and the activity registration, exactly as across a rotation,
 * and the loop keeps calling plugins (they hold the process-wide context, not a
 * view). The next activity is given the same id, which is how wry recognises it
 * and builds the webview into it again.
 */
class MainActivity : TauriActivity() {
  companion object {
    /** `WryActivity`'s private `ACTIVITY_ID_KEY`. */
    private const val WRY_ACTIVITY_ID_KEY = "__wryActivityId"

    /** Id of an activity that went away while the service kept the process. */
    @Volatile private var keptActivityId: Int? = null
  }

  private var keepRustWindow = false

  override fun onCreate(savedInstanceState: Bundle?) {
    keptActivityId?.let { kept ->
      keptActivityId = null
      // A restored activity already carries its id in the saved state.
      if (savedInstanceState == null) intent.putExtra(WRY_ACTIVITY_ID_KEY, kept)
    }
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }

  // tao and wry ask this (over JNI) from inside `super.onDestroy()`.
  override fun isChangingConfigurations(): Boolean =
    keepRustWindow || super.isChangingConfigurations()

  override fun onDestroy() {
    if (!super.isChangingConfigurations() && SyncForegroundService.isRunning) {
      keepRustWindow = true
      keptActivityId = id
    }
    super.onDestroy()
  }
}
