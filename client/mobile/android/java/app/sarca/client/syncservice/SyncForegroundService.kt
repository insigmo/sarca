package app.sarca.client.syncservice

import android.app.AlarmManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.database.ContentObserver
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.PowerManager
import android.os.SystemClock
import android.provider.MediaStore
import androidx.core.content.ContextCompat
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import java.util.Locale

/**
 * Keeps the Sarca process running while auto-upload is on, so Android does not
 * freeze it mid-upload. While a pass is moving files it also holds a partial
 * wake lock and a Wi-Fi lock, so the CPU and the radio stay up with the screen
 * off. Type `specialUse`: `dataSync` is capped at six hours a day on Android 15.
 *
 * Between passes it wakes the Rust loop: an alarm that fires through Doze, and
 * a watcher on the gallery. Rust's own timer runs on a clock that stops while
 * the device sleeps, so without these a 15-minute wait could last for hours.
 * The loop parks a call in [awaitWake]; [signalWake] answers it.
 *
 * Notifications: the service's own one stays up for as long as the service
 * runs — Android 14+ lets the user swipe it away, so its delete intent posts it
 * again — and shows how many files are done and left. A second one shows the
 * file on the wire and goes away when no file is.
 *
 * The service does no uploading itself; the Rust sync loop in the same process
 * does.
 */
class SyncForegroundService : Service() {
  companion object {
    private const val CHANNEL_ID = "sarca-sync"
    private const val FILE_CHANNEL_ID = "sarca-upload-file"
    private const val NOTIFICATION_ID = 0x5a7c
    private const val FILE_NOTIFICATION_ID = 0x5a7d
    private const val ACTION_REPOST = "app.sarca.client.syncservice.REPOST"
    private const val ACTION_WAKE = "app.sarca.client.syncservice.WAKE"
    /** Upper bound on one wake-lock hold; re-taken while a pass reports progress. */
    private const val WAKE_LOCK_TIMEOUT_MS = 60L * 60L * 1000L
    private const val WAKE_LOCK_REFRESH_MS = 5L * 60L * 1000L
    /** How long an alarm keeps the CPU up for the loop to start its pass. */
    private const val TICK_LOCK_MS = 2L * 60L * 1000L
    /** Matches the Rust loop's `BACKGROUND_POLL`. */
    private const val ALARM_INTERVAL_MS = 15L * 60L * 1000L
    /** A burst of photos is one wake, not one per photo. */
    private const val GALLERY_DEBOUNCE_MS = 10L * 1000L

    @Volatile private var instance: SyncForegroundService? = null
    @Volatile private var busy = false
    /** Set once Rust has asked for the service in this process. */
    @Volatile private var driven = false
    @Volatile private var progress = Progress()

    private val wakeGuard = Any()
    private var pendingWake: Invoke? = null
    private var wakeQueued = false

    /** Running for a Rust loop in this process; `MainActivity` keeps the process alive then. */
    val isRunning: Boolean
      get() = driven && instance != null

    fun start(context: Context) {
      driven = true
      ContextCompat.startForegroundService(
        context,
        Intent(context, SyncForegroundService::class.java),
      )
    }

    fun stop() {
      instance?.stopSelf()
    }

    fun setBusy(value: Boolean) {
      busy = value
      if (!value) progress = Progress()
      instance?.applyBusy()
    }

    fun setProgress(value: Progress) {
      // A late update can arrive after the pass ended; it must not bring the
      // file notification back.
      if (!busy) return
      progress = value
      instance?.applyProgress()
    }

    /** Parks the loop's call until the next wake; answers at once if one is queued. */
    fun awaitWake(invoke: Invoke) {
      var answerNow = false
      var replaced: Invoke? = null
      synchronized(wakeGuard) {
        if (wakeQueued) {
          wakeQueued = false
          answerNow = true
        } else {
          replaced = pendingWake
          pendingWake = invoke
        }
      }
      // A second wait replaced the first; let the first one go.
      replaced?.resolve(JSObject())
      if (answerNow) invoke.resolve(JSObject())
    }

    fun signalWake() {
      val waiting: Invoke?
      synchronized(wakeGuard) {
        waiting = pendingWake
        pendingWake = null
        if (waiting == null) wakeQueued = true
      }
      waiting?.resolve(JSObject())
    }
  }

  data class Progress(
    val done: Int = 0,
    val remaining: Int = 0,
    val total: Int = 0,
    val percent: Int = 0,
    val file: String? = null,
    val filePercent: Int = 0,
  )

  private var wakeLock: PowerManager.WakeLock? = null
  private var tickLock: PowerManager.WakeLock? = null
  private var wifiLock: WifiManager.WifiLock? = null
  private var lastLockRefresh = 0L
  private val handler = Handler(Looper.getMainLooper())
  private val galleryWake = Runnable { signalWake() }
  private val galleryObserver =
    object : ContentObserver(handler) {
      override fun onChange(selfChange: Boolean, uri: Uri?) {
        handler.removeCallbacks(galleryWake)
        handler.postDelayed(galleryWake, GALLERY_DEBOUNCE_MS)
      }
    }

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onCreate() {
    super.onCreate()
    instance = this
    try {
      contentResolver.registerContentObserver(
        MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
        true,
        galleryObserver,
      )
      contentResolver.registerContentObserver(
        MediaStore.Video.Media.EXTERNAL_CONTENT_URI,
        true,
        galleryObserver,
      )
    } catch (_: Exception) {
      // Without the media permission there is nothing to watch; the alarm
      // still wakes the loop.
    }
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    val notification = buildNotification()
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
      startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
    } else {
      startForeground(NOTIFICATION_ID, notification)
    }
    if (!driven) {
      // Started by a leftover alarm in a process where Rust never ran (the
      // old one was killed): there is no loop to wake, so do not linger with
      // a notification for work nobody is doing.
      cancelAlarm()
      stopSelf()
      return START_NOT_STICKY
    }
    when (intent?.action) {
      ACTION_WAKE -> {
        holdTickLock()
        signalWake()
      }
      ACTION_REPOST -> applyProgress()
    }
    scheduleAlarm()
    applyBusy()
    // Not sticky: a restarted service with no Rust loop behind it would only
    // show a notification for work nobody is doing.
    return START_NOT_STICKY
  }

  override fun onDestroy() {
    cancelAlarm()
    handler.removeCallbacks(galleryWake)
    try {
      contentResolver.unregisterContentObserver(galleryObserver)
    } catch (_: Exception) {}
    getSystemService(NotificationManager::class.java)?.cancel(FILE_NOTIFICATION_ID)
    releaseLocks()
    tickLock?.takeIf { it.isHeld }?.release()
    if (instance === this) instance = null
    super.onDestroy()
  }

  private fun applyBusy() {
    if (busy) acquireLocks() else releaseLocks()
    applyProgress()
  }

  private fun applyProgress() {
    val nm = getSystemService(NotificationManager::class.java) ?: return
    if (busy && SystemClock.elapsedRealtime() - lastLockRefresh >= WAKE_LOCK_REFRESH_MS) {
      // A pass longer than the lock's timeout must not lose the CPU halfway.
      acquireLocks()
    }
    nm.notify(NOTIFICATION_ID, buildNotification())
    val p = progress
    if (busy && p.file != null) {
      nm.notify(FILE_NOTIFICATION_ID, buildFileNotification(p.file, p.filePercent))
    } else {
      nm.cancel(FILE_NOTIFICATION_ID)
    }
  }

  private fun servicePendingIntent(action: String, requestCode: Int): PendingIntent =
    PendingIntent.getService(
      this,
      requestCode,
      Intent(this, SyncForegroundService::class.java).setAction(action),
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )

  private fun scheduleAlarm() {
    val am = getSystemService(AlarmManager::class.java) ?: return
    val at = SystemClock.elapsedRealtime() + ALARM_INTERVAL_MS
    val pi = servicePendingIntent(ACTION_WAKE, 2)
    // Inexact and allowed while idle: needs no exact-alarm permission, and
    // Doze still lets it through, at most every few minutes.
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
      am.setAndAllowWhileIdle(AlarmManager.ELAPSED_REALTIME_WAKEUP, at, pi)
    } else {
      am.set(AlarmManager.ELAPSED_REALTIME_WAKEUP, at, pi)
    }
  }

  private fun cancelAlarm() {
    getSystemService(AlarmManager::class.java)?.cancel(servicePendingIntent(ACTION_WAKE, 2))
  }

  private fun holdTickLock() {
    val lock =
      tickLock
        ?: getSystemService(PowerManager::class.java)
          ?.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "sarca:tick")
          ?.also {
            it.setReferenceCounted(false)
            tickLock = it
          }
    lock?.acquire(TICK_LOCK_MS)
  }

  private fun acquireLocks() {
    val wl =
      wakeLock
        ?: getSystemService(PowerManager::class.java)
          ?.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "sarca:upload")
          ?.also {
            it.setReferenceCounted(false)
            wakeLock = it
          }
    wl?.acquire(WAKE_LOCK_TIMEOUT_MS)
    lastLockRefresh = SystemClock.elapsedRealtime()

    if (wifiLock == null) {
      @Suppress("DEPRECATION")
      wifiLock =
        applicationContext
          .getSystemService(WifiManager::class.java)
          ?.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "sarca:upload")
          ?.also { it.setReferenceCounted(false) }
    }
    wifiLock?.takeIf { !it.isHeld }?.acquire()
  }

  private fun releaseLocks() {
    wakeLock?.takeIf { it.isHeld }?.release()
    wifiLock?.takeIf { it.isHeld }?.release()
  }

  private val russian: Boolean
    get() = Locale.getDefault().language == "ru"

  private fun ensureChannels(nm: NotificationManager) {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
    if (nm.getNotificationChannel(CHANNEL_ID) == null) {
      nm.createNotificationChannel(
        NotificationChannel(
          CHANNEL_ID,
          if (russian) "Автозагрузка" else "Auto-upload",
          NotificationManager.IMPORTANCE_LOW,
        ).apply {
          description =
            if (russian) {
              "Показывается, пока Sarca загружает фото в фоне"
            } else {
              "Shown while Sarca uploads photos in the background"
            }
          setShowBadge(false)
        },
      )
    }
    if (nm.getNotificationChannel(FILE_CHANNEL_ID) == null) {
      nm.createNotificationChannel(
        NotificationChannel(
          FILE_CHANNEL_ID,
          if (russian) "Загрузка файла" else "File upload",
          NotificationManager.IMPORTANCE_LOW,
        ).apply {
          description =
            if (russian) "Ход загрузки текущего файла" else "Progress of the file being uploaded"
          setShowBadge(false)
        },
      )
    }
  }

  private fun builder(channel: String): Notification.Builder {
    getSystemService(NotificationManager::class.java)?.let { ensureChannels(it) }
    return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      Notification.Builder(this, channel)
    } else {
      @Suppress("DEPRECATION") Notification.Builder(this)
    }
  }

  private fun openApp(): PendingIntent? =
    packageManager.getLaunchIntentForPackage(packageName)?.let {
      PendingIntent.getActivity(
        this,
        0,
        it,
        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
      )
    }

  private fun buildNotification(): Notification {
    val p = progress
    val counting = busy && p.total > 0
    val title =
      when {
        counting -> if (russian) "Автозагрузка · ${p.percent}%" else "Auto-upload · ${p.percent}%"
        else -> "Sarca"
      }
    val text =
      when {
        counting ->
          if (russian) {
            "Загружено ${p.done} из ${p.total} · осталось ${p.remaining}"
          } else {
            "Uploaded ${p.done} of ${p.total} · ${p.remaining} left"
          }
        busy -> if (russian) "Ищу новые фото и видео" else "Looking for new photos and videos"
        else -> if (russian) "Автозагрузка включена" else "Auto-upload is on"
      }
    val b =
      builder(CHANNEL_ID)
        .setSmallIcon(
          if (busy) android.R.drawable.stat_sys_upload else android.R.drawable.stat_notify_sync_noanim,
        )
        .setContentTitle(title)
        .setContentText(text)
        .setOngoing(true)
        .setOnlyAlertOnce(true)
        .setShowWhen(false)
        .setContentIntent(openApp())
        // Android 14+ lets the user swipe this away; post it straight back.
        .setDeleteIntent(servicePendingIntent(ACTION_REPOST, 1))
    if (counting) b.setProgress(100, p.percent, false)
    val n = b.build()
    n.flags = n.flags or Notification.FLAG_NO_CLEAR or Notification.FLAG_ONGOING_EVENT
    return n
  }

  private fun buildFileNotification(name: String, percent: Int): Notification =
    builder(FILE_CHANNEL_ID)
      .setSmallIcon(android.R.drawable.stat_sys_upload)
      .setContentTitle(name)
      .setContentText(if (russian) "Загрузка на сервер · $percent%" else "Uploading to the server · $percent%")
      .setProgress(100, percent, false)
      .setOngoing(true)
      .setOnlyAlertOnce(true)
      .setShowWhen(false)
      .setCategory(Notification.CATEGORY_PROGRESS)
      .setContentIntent(openApp())
      .build()
}
