package com.translator.subtitleoverlay

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.graphics.PixelFormat
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.TypedValue
import android.view.Gravity
import android.view.LayoutInflater
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.widget.LinearLayout
import android.widget.TextView
import androidx.core.app.NotificationCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope

/**
 * 浮窗字幕服務 — 前景服務，使用 WindowManager 在其他 App 上方顯示字幕
 */
class OverlayService : LifecycleService(), SseClient.EventListener {

    /** Log 回呼介面，供主畫面接收偵錯訊息 */
    interface LogCallback {
        fun onLog(message: String)
    }

    companion object {
        const val CHANNEL_ID = "subtitle_overlay_channel"
        const val NOTIFICATION_ID = 1001
        const val ACTION_STOP = "com.translator.subtitleoverlay.ACTION_STOP"

        const val EXTRA_SERVER_HOST = "server_host"
        const val EXTRA_MAIN_PORT = "main_port"
        const val EXTRA_PUBLIC_PORT = "public_port"

        /** 靜態參照，供 MainActivity 存取 */
        var instance: OverlayService? = null
            private set
    }

    var logCallback: LogCallback? = null

    private lateinit var windowManager: WindowManager
    private lateinit var overlayView: View
    private lateinit var subtitleContainer: LinearLayout
    private lateinit var subtitleScroll: android.widget.ScrollView
    private lateinit var waitingText: TextView
    private lateinit var settingsManager: SettingsManager
    private var sseClient: SseClient? = null
    private val handler = Handler(Looper.getMainLooper())
    private val subtitleEntries = mutableListOf<SubtitleEntry>()

    override fun onCreate() {
        super.onCreate()
        instance = this
        settingsManager = SettingsManager(this)
        windowManager = getSystemService(WINDOW_SERVICE) as WindowManager

        createNotificationChannel()
        startForeground(NOTIFICATION_ID, buildNotification("準備中..."))
        createOverlayView()
        emitLog("服務已建立")
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)

        when (intent?.action) {
            ACTION_STOP -> {
                stopSelf()
                return START_NOT_STICKY
            }
        }

        // 從 Intent 取得連線參數
        val host = intent?.getStringExtra(EXTRA_SERVER_HOST) ?: settingsManager.serverHost
        val mainPort = intent?.getIntExtra(EXTRA_MAIN_PORT, settingsManager.mainPort) ?: settingsManager.mainPort
        val publicPort = intent?.getIntExtra(EXTRA_PUBLIC_PORT, settingsManager.publicPort) ?: settingsManager.publicPort

        // 啟動 SSE 連線
        startSseConnection(host, mainPort, publicPort)

        return START_STICKY
    }

    override fun onDestroy() {
        sseClient?.stop()
        try {
            windowManager.removeView(overlayView)
        } catch (_: Exception) { }
        instance = null
        emitLog("服務已銷毀")
        super.onDestroy()
    }

    override fun onBind(intent: Intent): IBinder? {
        super.onBind(intent)
        return null
    }

    // === 建立浮窗 ===

    /** 邊緣觸控區域寬度 (dp) */
    private val EDGE_THRESHOLD_DP = 24f
    /** 最小浮窗尺寸 (dp) */
    private val MIN_SIZE_DP = 120f

    private lateinit var layoutParams: WindowManager.LayoutParams
    private lateinit var resizeBorder: View

    /** 觸控模式 */
    private enum class TouchMode { NONE, MOVE, RESIZE }

    /** 拖曳調整的邊 */
    private data class ResizeEdges(
        val left: Boolean = false,
        val top: Boolean = false,
        val right: Boolean = false,
        val bottom: Boolean = false
    )

    private fun createOverlayView() {
        overlayView = LayoutInflater.from(this).inflate(R.layout.overlay_subtitle, null)
        subtitleContainer = overlayView.findViewById(R.id.subtitle_container)
        subtitleScroll = overlayView.findViewById(R.id.subtitle_scroll)
        waitingText = overlayView.findViewById(R.id.text_waiting)
        resizeBorder = overlayView.findViewById(R.id.resize_border)
        val rootView = overlayView.findViewById<android.widget.FrameLayout>(R.id.overlay_root)
        val touchCatcher = overlayView.findViewById<View>(R.id.touch_catcher)

        // 套用背景色
        rootView.setBackgroundColor(settingsManager.getBackgroundColorWithAlpha())

        // 初始時顯示等待提示
        waitingText.visibility = View.VISIBLE
        subtitleContainer.visibility = View.GONE

        // 設定 resize border drawable
        resizeBorder.setBackgroundResource(R.drawable.resize_border)

        // 設定 resize handle drawable
        overlayView.findViewById<View>(R.id.resize_handle)
            ?.setBackgroundResource(R.drawable.resize_handle)

        // 計算初始尺寸
        val dm = resources.displayMetrics
        val defaultWidth = (dm.widthPixels * 0.8).toInt()
        val defaultHeight = (200 * dm.density).toInt()
        val width = if (settingsManager.overlayWidth > 0) settingsManager.overlayWidth else defaultWidth
        val height = if (settingsManager.overlayHeight > 0) settingsManager.overlayHeight else defaultHeight

        layoutParams = WindowManager.LayoutParams(
            width,
            height,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
                    WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL or
                    WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN,
            PixelFormat.TRANSLUCENT
        ).apply {
            // 使用 TOP|START 簡化座標計算
            gravity = Gravity.TOP or Gravity.START
            x = (dm.widthPixels - width) / 2  // 水平置中
            y = dm.heightPixels - height - (48 * dm.density).toInt()  // 靠近底部
        }

        // 觸控監聽設在最上層的 touch_catcher，避免 ScrollView 攔截
        setupTouchListener(touchCatcher)

        windowManager.addView(overlayView, layoutParams)
    }

    /**
     * 統一觸控處理 — 根據觸控位置自動判斷移動或調整大小
     *
     * 邊緣區域 (24dp 內) → 調整大小
     * 中央區域 → 拖曳移動
     */
    private fun setupTouchListener(touchTarget: View) {
        val dm = resources.displayMetrics
        val edgeThreshold = (EDGE_THRESHOLD_DP * dm.density).toInt()
        val minSize = (MIN_SIZE_DP * dm.density).toInt()

        var touchMode = TouchMode.NONE
        var resizeEdges = ResizeEdges()

        // 移動用
        var initialX = 0
        var initialY = 0
        var initialTouchX = 0f
        var initialTouchY = 0f

        // 調整大小用
        var initialW = 0
        var initialH = 0

        touchTarget.setOnTouchListener { _, event ->
            when (event.action) {
                MotionEvent.ACTION_DOWN -> {
                    val touchX = event.x.toInt()
                    val touchY = event.y.toInt()
                    val viewW = overlayView.width
                    val viewH = overlayView.height

                    // 判斷觸控是否在邊緣
                    val nearLeft = touchX < edgeThreshold
                    val nearRight = touchX > viewW - edgeThreshold
                    val nearTop = touchY < edgeThreshold
                    val nearBottom = touchY > viewH - edgeThreshold
                    val onEdge = nearLeft || nearRight || nearTop || nearBottom

                    if (onEdge) {
                        touchMode = TouchMode.RESIZE
                        resizeEdges = ResizeEdges(nearLeft, nearTop, nearRight, nearBottom)
                        resizeBorder.visibility = View.VISIBLE
                    } else {
                        touchMode = TouchMode.MOVE
                    }

                    initialX = layoutParams.x
                    initialY = layoutParams.y
                    initialW = layoutParams.width
                    initialH = layoutParams.height
                    initialTouchX = event.rawX
                    initialTouchY = event.rawY
                    true
                }
                MotionEvent.ACTION_MOVE -> {
                    val dx = (event.rawX - initialTouchX).toInt()
                    val dy = (event.rawY - initialTouchY).toInt()

                    when (touchMode) {
                        TouchMode.MOVE -> {
                            layoutParams.x = initialX + dx
                            layoutParams.y = initialY + dy
                            windowManager.updateViewLayout(overlayView, layoutParams)
                        }
                        TouchMode.RESIZE -> {
                            if (resizeEdges.right) {
                                layoutParams.width = (initialW + dx).coerceAtLeast(minSize)
                            }
                            if (resizeEdges.bottom) {
                                layoutParams.height = (initialH + dy).coerceAtLeast(minSize)
                            }
                            if (resizeEdges.left) {
                                val newW = (initialW - dx).coerceAtLeast(minSize)
                                layoutParams.x = initialX + (initialW - newW)
                                layoutParams.width = newW
                            }
                            if (resizeEdges.top) {
                                val newH = (initialH - dy).coerceAtLeast(minSize)
                                layoutParams.y = initialY + (initialH - newH)
                                layoutParams.height = newH
                            }
                            windowManager.updateViewLayout(overlayView, layoutParams)
                        }
                        else -> {}
                    }
                    true
                }
                MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                    if (touchMode == TouchMode.RESIZE) {
                        settingsManager.overlayWidth = layoutParams.width
                        settingsManager.overlayHeight = layoutParams.height
                        resizeBorder.visibility = View.GONE
                    }
                    touchMode = TouchMode.NONE
                    true
                }
                else -> false
            }
        }
    }

    // === SSE 連線 ===

    private fun startSseConnection(host: String, mainPort: Int, publicPort: Int) {
        sseClient?.stop()
        sseClient = SseClient(host, mainPort, publicPort).apply {
            setEventListener(this@OverlayService)
            start(lifecycleScope)
        }
    }

    // === SseClient.EventListener 回呼 ===

    override fun onSubtitle(entry: SubtitleEntry) {
        handler.post {
            val countBefore = subtitleEntries.size
            addSubtitleToView(entry)
            val action = if (subtitleEntries.size > countBefore) "新增" else "更新"
            updateNotification("字幕串流中...")
            emitLog("[字幕/$action] ts=${entry.timestamp} | ${entry.original} | ${entry.translated}")
        }
    }

    override fun onStatus(status: String, pid: Int?, code: Int?) {
        handler.post {
            when (status) {
                "completed" -> updateNotification("任務已完成")
                "running" -> updateNotification("字幕串流中...")
            }
            emitLog("[狀態] status=$status pid=$pid code=$code")
        }
    }

    override fun onError(message: String) {
        handler.post {
            updateNotification("錯誤: $message")
            emitLog("[錯誤] $message")
        }
    }

    override fun onConnectionStateChanged(state: ConnectionState) {
        handler.post {
            val text = when (state) {
                ConnectionState.CONNECTING -> "連線中..."
                ConnectionState.CONNECTED -> "已連線"
                ConnectionState.RECONNECTING -> "重新連線中..."
                ConnectionState.DISCONNECTED -> "已斷線"
                ConnectionState.ERROR -> "連線錯誤"
            }
            updateNotification(text)
            emitLog("[連線] $text")
        }
    }

    // === 字幕顯示邏輯 ===

    private fun addSubtitleToView(entry: SubtitleEntry) {
        // 有字幕進來時切換為字幕模式
        waitingText.visibility = View.GONE
        subtitleContainer.visibility = View.VISIBLE

        // === 去重與更新邏輯 ===
        val updated = tryUpdateExisting(entry)
        if (!updated) {
            // 全新字幕，追加
            subtitleEntries.add(entry)

            // 超過上限時移除最舊的字幕
            val maxCount = settingsManager.maxSubtitleCount
            while (subtitleEntries.size > maxCount) {
                subtitleEntries.removeAt(0)
            }
        }

        // 重建字幕視圖
        rebuildSubtitleViews()
    }

    /**
     * 嘗試更新已存在的字幕條目（去重邏輯）
     *
     * 規則：
     * 1. 若 timestamp 非空且與某條目相同 → 更新該條目（漸進式辨識的更新）
     * 2. 若 original 與最後一條完全相同 → 更新翻譯（可能是翻譯延後到達）
     * 3. 否則視為新字幕，回傳 false
     */
    private fun tryUpdateExisting(entry: SubtitleEntry): Boolean {
        // 策略一：timestamp 匹配 → 更新（SSE 漸進式辨識常見情境）
        if (entry.timestamp.isNotEmpty()) {
            val idx = subtitleEntries.indexOfLast { it.timestamp == entry.timestamp }
            if (idx >= 0) {
                subtitleEntries[idx] = entry
                return true
            }
        }

        // 策略二：原文完全相同於最後一條 → 更新翻譯
        if (subtitleEntries.isNotEmpty() && entry.original.isNotEmpty()) {
            val last = subtitleEntries.last()
            if (last.original == entry.original) {
                // 翻譯可能有更新，覆蓋
                subtitleEntries[subtitleEntries.lastIndex] = entry
                return true
            }
        }

        return false
    }

    private fun rebuildSubtitleViews() {
        subtitleContainer.removeAllViews()

        val fontSize = settingsManager.fontSize
        val originalColor = settingsManager.originalTextColor
        val translatedColor = settingsManager.translatedTextColor

        for ((index, entry) in subtitleEntries.withIndex()) {
            // 每個字幕項目的容器
            val entryLayout = LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(12, 4, 12, 4)
            }

            // 原文
            if (entry.original.isNotEmpty()) {
                val originalView = TextView(this).apply {
                    text = entry.original
                    setTextColor(originalColor)
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, fontSize)
                    setShadowLayer(2f, 1f, 1f, Color.BLACK)
                }
                entryLayout.addView(originalView)
            }

            // 翻譯
            if (entry.translated.isNotEmpty()) {
                val translatedView = TextView(this).apply {
                    text = entry.translated
                    setTextColor(translatedColor)
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, fontSize)
                    setShadowLayer(2f, 1f, 1f, Color.BLACK)
                }
                entryLayout.addView(translatedView)
            }

            subtitleContainer.addView(entryLayout)

            // 字幕之間加分隔線（除了最後一個）
            if (index < subtitleEntries.size - 1) {
                val divider = View(this).apply {
                    layoutParams = LinearLayout.LayoutParams(
                        LinearLayout.LayoutParams.MATCH_PARENT, 1
                    ).apply { setMargins(12, 2, 12, 2) }
                    setBackgroundColor(Color.argb(60, 255, 255, 255))
                }
                subtitleContainer.addView(divider)
            }
        }

        // 更新背景色
        overlayView.findViewById<android.widget.FrameLayout>(R.id.overlay_root)
            ?.setBackgroundColor(settingsManager.getBackgroundColorWithAlpha())

        // 自動滾到底部（最新字幕在最下方）
        subtitleScroll.post {
            subtitleScroll.fullScroll(View.FOCUS_DOWN)
        }
    }

    /** 動態更新外觀設定（由 MainActivity 呼叫） */
    fun refreshAppearance() {
        handler.post { rebuildSubtitleViews() }
    }

    // === 通知管理 ===

    private fun createNotificationChannel() {
        val channel = NotificationChannel(
            CHANNEL_ID,
            "字幕浮窗服務",
            NotificationManager.IMPORTANCE_LOW
        ).apply {
            description = "顯示浮窗字幕的前景服務通知"
            setShowBadge(false)
        }
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(channel)
    }

    private fun buildNotification(text: String): Notification {
        // 點擊通知開啟主畫面
        val openIntent = Intent(this, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP
        }
        val openPending = PendingIntent.getActivity(
            this, 0, openIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        // 停止按鈕
        val stopIntent = Intent(this, OverlayService::class.java).apply {
            action = ACTION_STOP
        }
        val stopPending = PendingIntent.getService(
            this, 1, stopIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("浮窗字幕")
            .setContentText(text)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentIntent(openPending)
            .addAction(android.R.drawable.ic_delete, "停止", stopPending)
            .setOngoing(true)
            .setSilent(true)
            .build()
    }

    private fun updateNotification(text: String) {
        val manager = getSystemService(NotificationManager::class.java)
        manager.notify(NOTIFICATION_ID, buildNotification(text))
    }

    /** 發送 log 訊息至主畫面 */
    private fun emitLog(message: String) {
        val timestamp = java.text.SimpleDateFormat("HH:mm:ss", java.util.Locale.getDefault()).format(java.util.Date())
        logCallback?.onLog("[$timestamp] $message")
    }
}
