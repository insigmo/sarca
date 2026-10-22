package app.sarca.client.syncservice

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.content.ContextCompat

/**
 * Keeps the Sarca process running while auto-upload is on, so Android does not
 * freeze it mid-upload. While a pass is moving files it also holds a partial
 * wake lock and a Wi-Fi lock, so the CPU and the radio stay up with the screen
 * off. Type `specialUse`: `dataSync` is capped at six hours a day on Android 15.
 *
 * The service does no work itself; the Rust sync loop in the same process does.
 */
class SyncForegroundService : Service() {
  companion object {
    private const val CHANNEL_ID = "sarca-sync"
    private const val NOTIFICATION_ID = 0x5a7c
    /** Upper bound on one wake-lock hold; re-taken when the next pass starts. */
    private const val WAKE_LOCK_TIMEOUT_MS = 60L * 60L * 1000L

    @Volatile private var instance: SyncForegroundService? = null
    @Volatile private var busy = false

    fun start(context: Context) {
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
      instance?.applyBusy()
    }
  }

  private var wakeLock: PowerManager.WakeLock? = null
  private var wifiLock: WifiManager.WifiLock? = null

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onCreate() {
    super.onCreate()
    instance = this
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    val notification = buildNotification()
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
      startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
    } else {
      startForeground(NOTIFICATION_ID, notification)
    }
    applyBusy()
    // Not sticky: a restarted service with no Rust loop behind it would only
    // show a notification for work nobody is doing.
    return START_NOT_STICKY
  }

  override fun onDestroy() {
    releaseLocks()
    if (instance === this) instance = null
    super.onDestroy()
  }

  private fun applyBusy() {
    if (busy) acquireLocks() else releaseLocks()
    getSystemService(NotificationManager::class.java)?.notify(NOTIFICATION_ID, buildNotification())
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

  private fun buildNotification(): Notification {
    val nm = getSystemService(NotificationManager::class.java)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O &&
      nm?.getNotificationChannel(CHANNEL_ID) == null
    ) {
      nm?.createNotificationChannel(
        NotificationChannel(CHANNEL_ID, "Auto-upload", NotificationManager.IMPORTANCE_LOW).apply {
          description = "Shown while Sarca uploads photos in the background"
          setShowBadge(false)
        },
      )
    }
    val open =
      packageManager.getLaunchIntentForPackage(packageName)?.let {
        PendingIntent.getActivity(
          this,
          0,
          it,
          PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
      }
    val builder =
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
        Notification.Builder(this, CHANNEL_ID)
      } else {
        @Suppress("DEPRECATION") Notification.Builder(this)
      }
    return builder
      .setSmallIcon(
        if (busy) android.R.drawable.stat_sys_upload else android.R.drawable.stat_notify_sync_noanim,
      )
      .setContentTitle("Sarca")
      .setContentText(if (busy) "Uploading photos and videos" else "Auto-upload is on")
      .setOngoing(true)
      .setOnlyAlertOnce(true)
      .setContentIntent(open)
      .build()
  }
}
