package app.sarca.client.folderpicker

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.DocumentsContract
import androidx.activity.result.ActivityResult
import app.tauri.Logger
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

/**
 * SAF folder picker (ACTION_OPEN_DOCUMENT_TREE).
 * Resolves primary/external-storage tree URIs to filesystem paths the sync
 * engine can walk with READ_MEDIA_* / READ_EXTERNAL_STORAGE.
 */
@TauriPlugin
class FolderPickerPlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun pickFolder(invoke: Invoke) {
    try {
      val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
      intent.addFlags(
        Intent.FLAG_GRANT_READ_URI_PERMISSION or
          Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
      )
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
        intent.putExtra(
          DocumentsContract.EXTRA_INITIAL_URI,
          Uri.parse("content://com.android.externalstorage.documents/document/primary:DCIM"),
        )
      }
      startActivityForResult(invoke, intent, "folderPickerResult")
    } catch (ex: Exception) {
      val message = ex.message ?: "Failed to open folder picker"
      Logger.error(message)
      invoke.reject(message)
    }
  }

  @ActivityCallback
  fun folderPickerResult(invoke: Invoke, result: ActivityResult) {
    try {
      when (result.resultCode) {
        Activity.RESULT_OK -> {
          val uri = result.data?.data
          if (uri == null) {
            invoke.resolve(nullResult())
            return
          }
          try {
            activity.contentResolver.takePersistableUriPermission(
              uri,
              Intent.FLAG_GRANT_READ_URI_PERMISSION,
            )
          } catch (ex: SecurityException) {
            Logger.error("takePersistableUriPermission: ${ex.message}")
          }
          val path = treeUriToPath(uri)
          val ret = JSObject()
          ret.put("path", path)
          ret.put("uri", uri.toString())
          invoke.resolve(ret)
        }
        Activity.RESULT_CANCELED -> invoke.resolve(nullResult())
        else -> invoke.reject("Failed to pick folder")
      }
    } catch (ex: Exception) {
      val message = ex.message ?: "Failed to read folder pick result"
      Logger.error(message)
      invoke.reject(message)
    }
  }

  private fun nullResult(): JSObject {
    val ret = JSObject()
    ret.put("path", null)
    ret.put("uri", null)
    return ret
  }

  /**
   * Convert common SAF tree URIs to absolute paths.
   * primary:DCIM → /storage/emulated/0/DCIM
   * <uuid>:DCIM → /storage/<uuid>/DCIM
   */
  private fun treeUriToPath(uri: Uri): String? {
    val docId =
      try {
        DocumentsContract.getTreeDocumentId(uri)
      } catch (_: Exception) {
        return null
      }
    val split = docId.split(":", limit = 2)
    if (split.isEmpty()) return null
    val volume = split[0]
    val relative = if (split.size > 1) split[1].trim('/') else ""
    val root =
      when {
        volume.equals("primary", ignoreCase = true) -> "/storage/emulated/0"
        volume.matches(Regex("[A-Fa-f0-9]{4}-[A-Fa-f0-9]{4}")) -> "/storage/$volume"
        else -> return null
      }
    return if (relative.isEmpty()) root else "$root/$relative"
  }
}
