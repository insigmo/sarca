package app.sarca.client.mediastore

import android.app.Activity
import android.content.ContentUris
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.io.IOException
import java.util.UUID

/**
 * MediaStore-backed DCIM discovery and URI materialization for auto-upload.
 * Installed into the generated Android project by scripts/patch-android-http.sh.
 */
@TauriPlugin
class MediaStorePlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun listDcimMedia(invoke: Invoke) {
    try {
      val items = JSArray()
      appendAll(items, queryCollection(MediaStore.Images.Media.EXTERNAL_CONTENT_URI))
      appendAll(items, queryCollection(MediaStore.Video.Media.EXTERNAL_CONTENT_URI))
      val ret = JSObject()
      ret.put("items", items)
      invoke.resolve(ret)
    } catch (ex: SecurityException) {
      invoke.reject(ex.message ?: "MediaStore permission denied")
    } catch (ex: Exception) {
      invoke.reject(ex.message ?: "MediaStore query failed")
    }
  }

  @Command
  fun materializeForUpload(invoke: Invoke) {
    try {
      val args = invoke.getArgs()
      val uri = Uri.parse(args.getString("uri"))
      val direct = tryDirectPath(uri)
      val ret = JSObject()
      if (direct != null) {
        ret.put("path", direct)
        ret.put("ephemeral", false)
      } else {
        ret.put("path", copyToUploadCache(uri))
        ret.put("ephemeral", true)
      }
      invoke.resolve(ret)
    } catch (ex: Exception) {
      invoke.reject(ex.message ?: "materialize failed")
    }
  }

  private fun queryCollection(collectionUri: Uri): JSArray {
    val items = JSArray()
    val resolver = activity.contentResolver
    val projection =
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
        arrayOf(
          MediaStore.MediaColumns._ID,
          MediaStore.MediaColumns.DISPLAY_NAME,
          MediaStore.MediaColumns.RELATIVE_PATH,
          MediaStore.MediaColumns.SIZE,
          MediaStore.MediaColumns.DATE_MODIFIED,
        )
      } else {
        @Suppress("DEPRECATION")
        arrayOf(
          MediaStore.MediaColumns._ID,
          MediaStore.MediaColumns.DISPLAY_NAME,
          MediaStore.MediaColumns.DATA,
          MediaStore.MediaColumns.SIZE,
          MediaStore.MediaColumns.DATE_MODIFIED,
        )
      }

    resolver.query(collectionUri, projection, null, null, null)?.use { cursor ->
      val idIdx = cursor.getColumnIndex(MediaStore.MediaColumns._ID)
      val nameIdx = cursor.getColumnIndex(MediaStore.MediaColumns.DISPLAY_NAME)
      val relIdx =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
          cursor.getColumnIndex(MediaStore.MediaColumns.RELATIVE_PATH)
        } else {
          -1
        }
      @Suppress("DEPRECATION")
      val dataIdx =
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
          cursor.getColumnIndex(MediaStore.MediaColumns.DATA)
        } else {
          -1
        }
      val sizeIdx = cursor.getColumnIndex(MediaStore.MediaColumns.SIZE)
      val mtimeIdx = cursor.getColumnIndex(MediaStore.MediaColumns.DATE_MODIFIED)

      while (cursor.moveToNext()) {
        val relativePath =
          when {
            relIdx >= 0 -> cursor.getString(relIdx)
            dataIdx >= 0 -> relativePathFromData(cursor.getString(dataIdx))
            else -> null
          }
        if (!isDcimRelative(relativePath)) continue

        val id = cursor.getLong(idIdx)
        val displayName = cursor.getString(nameIdx) ?: continue
        val size = if (sizeIdx >= 0 && !cursor.isNull(sizeIdx)) cursor.getLong(sizeIdx) else 0L
        if (size <= 0L) continue

        val dateModifiedSec =
          if (mtimeIdx >= 0 && !cursor.isNull(mtimeIdx)) cursor.getLong(mtimeIdx) else 0L

        val itemUri = ContentUris.withAppendedId(collectionUri, id)
        val item = JSObject()
        item.put("uri", itemUri.toString())
        item.put("displayName", displayName)
        item.put("relativePath", relativePath)
        item.put("size", size)
        item.put("dateModifiedMs", dateModifiedSec * 1000)
        items.put(item)
      }
    }
    return items
  }

  private fun tryDirectPath(uri: Uri): String? {
    @Suppress("DEPRECATION")
    val projection = arrayOf(MediaStore.MediaColumns.DATA)
    activity.contentResolver.query(uri, projection, null, null, null)?.use { cursor ->
      if (!cursor.moveToFirst()) return null
      val dataIdx = cursor.getColumnIndex(MediaStore.MediaColumns.DATA)
      if (dataIdx < 0) return null
      val path = cursor.getString(dataIdx) ?: return null
      if (path.isBlank()) return null
      val file = File(path)
      return if (file.isFile && file.canRead()) path else null
    }
    return null
  }

  private fun copyToUploadCache(uri: Uri): String {
    val uploadDir = File(activity.cacheDir, "sarca-upload")
    if (!uploadDir.exists() && !uploadDir.mkdirs()) {
      throw IOException("Failed to create upload cache directory")
    }
    val displayName = queryDisplayName(uri) ?: "upload"
    val safeName = displayName.replace(Regex("[^a-zA-Z0-9._-]"), "_")
    val dest = File(uploadDir, "${UUID.randomUUID()}_$safeName")
    activity.contentResolver.openInputStream(uri)?.use { input ->
      dest.outputStream().use { output -> input.copyTo(output) }
    } ?: throw IOException("Cannot open content URI for read")
    return dest.absolutePath
  }

  private fun queryDisplayName(uri: Uri): String? {
    val projection = arrayOf(MediaStore.MediaColumns.DISPLAY_NAME)
    activity.contentResolver.query(uri, projection, null, null, null)?.use { cursor ->
      if (cursor.moveToFirst()) {
        val idx = cursor.getColumnIndex(MediaStore.MediaColumns.DISPLAY_NAME)
        if (idx >= 0) return cursor.getString(idx)
      }
    }
    return null
  }

  private fun relativePathFromData(dataPath: String?): String? {
    if (dataPath.isNullOrBlank()) return null
    val normalized = dataPath.replace('\\', '/')
    val dcimIdx = normalized.indexOf("/DCIM/")
    if (dcimIdx < 0) {
      return if (normalized.endsWith("/DCIM")) "DCIM/" else null
    }
    val tail = normalized.substring(dcimIdx + 1)
    val lastSlash = tail.lastIndexOf('/')
    return if (lastSlash >= 0) tail.substring(0, lastSlash + 1) else "DCIM/"
  }

  private fun isDcimRelative(rel: String?): Boolean {
    if (rel == null) return false
    val n = rel.trimStart('/')
    return n == "DCIM" || n == "DCIM/" || n.startsWith("DCIM/")
  }

  private fun appendAll(into: JSArray, from: JSArray) {
    for (i in 0 until from.length()) {
      into.put(from.get(i))
    }
  }
}
