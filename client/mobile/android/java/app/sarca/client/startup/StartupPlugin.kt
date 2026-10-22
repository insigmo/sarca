package app.sarca.client.startup

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.OpenableColumns
import android.provider.Settings
import androidx.activity.result.ActivityResult
import androidx.core.content.ContextCompat
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

/**
 * Runtime media + battery-opt prompts, device model, and the log export.
 * Installed into the generated Android project by scripts/patch-android-http.sh.
 */
@TauriPlugin
class StartupPlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun ensureRuntimeAccess(invoke: Invoke) {
    // Fire-and-forget: the battery-opt exemption is a nice-to-have and must
    // never block the media-permission wait below — callers (the Rust side,
    // and in turn the first sync tick) rely on this command not resolving
    // until the user has actually answered the media permission prompt.
    requestIgnoreBatteryOptimizations()

    // Notifications ride along: without them Android 13+ hides the
    // auto-upload service's notification, and the user cannot see it running.
    val wanted =
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        mediaPermissions() + Manifest.permission.POST_NOTIFICATIONS
      } else {
        mediaPermissions()
      }
    val missing =
      wanted.filter {
        ContextCompat.checkSelfPermission(activity, it) != PackageManager.PERMISSION_GRANTED
      }
    if (missing.isEmpty()) {
      invoke.resolve(JSObject())
      return
    }
    // Uses Tauri's permission-request plumbing (backed by
    // registerForActivityResult under the hood) so the invoke only resolves
    // once the system dialog has been answered — either granted or denied.
    handle!!.requestPermissions(invoke, missing.toTypedArray(), "onMediaPermissionResult")
  }

  @PermissionCallback
  fun onMediaPermissionResult(invoke: Invoke) {
    invoke.resolve(JSObject())
  }

  /** Non-prompting check of current media permission state. */
  @Command
  fun hasMediaPermission(invoke: Invoke) {
    val granted =
      mediaPermissions().all {
        ContextCompat.checkSelfPermission(activity, it) == PackageManager.PERMISSION_GRANTED
      }
    val ret = JSObject()
    ret.put("granted", granted)
    invoke.resolve(ret)
  }

  @Command
  fun deviceModel(invoke: Invoke) {
    val ret = JSObject()
    val model = (Build.MODEL ?: "").trim()
    val manufacturer = (Build.MANUFACTURER ?: "").trim()
    ret.put("model", model)
    ret.put("manufacturer", manufacturer)
    val label =
      when {
        model.isEmpty() -> manufacturer
        manufacturer.isEmpty() -> model
        model.startsWith(manufacturer, ignoreCase = true) -> model
        else -> "$manufacturer $model"
      }
    ret.put("label", label.trim())
    invoke.resolve(ret)
  }

  /**
   * Asks the user where to save `text` (the system "Save as" screen, which
   * also offers Downloads, Drive and other document providers) and writes it
   * there. Resolves `{ saved: false }` when the user backs out.
   */
  @Command
  fun saveTextFile(invoke: Invoke) {
    try {
      // Invoke has getArgs()/parseArgs — not getString (that lives on JSObject).
      val name = invoke.getArgs().getString("fileName", "sarca-client.log") ?: "sarca-client.log"
      val intent =
        Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
          addCategory(Intent.CATEGORY_OPENABLE)
          type = "text/plain"
          putExtra(Intent.EXTRA_TITLE, name)
        }
      startActivityForResult(invoke, intent, "saveTextFileResult")
    } catch (ex: Exception) {
      invoke.reject(ex.message ?: "could not open the save screen")
    }
  }

  @ActivityCallback
  fun saveTextFileResult(invoke: Invoke, result: ActivityResult) {
    val ret = JSObject()
    try {
      val uri = result.data?.data
      if (result.resultCode != Activity.RESULT_OK || uri == null) {
        ret.put("saved", false)
        invoke.resolve(ret)
        return
      }
      val text = invoke.getArgs().getString("text", "") ?: ""
      val out =
        activity.contentResolver.openOutputStream(uri, "wt")
          ?: throw IllegalStateException("cannot write to the chosen file")
      out.use { it.write(text.toByteArray(Charsets.UTF_8)) }
      ret.put("saved", true)
      ret.put("name", displayName(uri) ?: uri.lastPathSegment ?: "")
      invoke.resolve(ret)
    } catch (ex: Exception) {
      invoke.reject(ex.message ?: "saving the log failed")
    }
  }

  private fun displayName(uri: Uri): String? =
    try {
      activity.contentResolver
        .query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
        ?.use { if (it.moveToFirst()) it.getString(0) else null }
    } catch (_: Exception) {
      null
    }

  private fun mediaPermissions(): Array<String> {
    return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      arrayOf(
        Manifest.permission.READ_MEDIA_IMAGES,
        Manifest.permission.READ_MEDIA_VIDEO,
      )
    } else {
      arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE)
    }
  }

  private fun requestIgnoreBatteryOptimizations() {
    try {
      val pm = activity.getSystemService(PowerManager::class.java) ?: return
      if (pm.isIgnoringBatteryOptimizations(activity.packageName)) return
      val intent =
        Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS).apply {
          data = Uri.parse("package:${activity.packageName}")
        }
      activity.startActivity(intent)
    } catch (_: Exception) {
      try {
        val fallback = Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)
        activity.startActivity(fallback)
      } catch (_: Exception) {
        // ignore — user can still change this in system settings
      }
    }
  }
}
