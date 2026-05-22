package com.translator.subtitleoverlay

import kotlinx.coroutines.*
import okhttp3.*
import org.json.JSONObject
import java.io.BufferedReader
import java.io.InputStreamReader
import java.util.concurrent.TimeUnit

/**
 * SSE 客戶端 — 負責與翻譯伺服器建立 Server-Sent Events 連線並解析事件
 *
 * 連線流程：
 * 1. GET /api/translation/active-task → 取得當前任務 ID
 * 2. GET /api/translation/stream/{task_id} → 開始 SSE 串流
 */
class SseClient(
    baseUrl: String
) {

    /** 去除尾部斜線的 base URL */
    private val baseUrl = baseUrl.trimEnd('/')

    /** SSE 事件監聽介面 */
    interface EventListener {
        fun onSubtitle(entry: SubtitleEntry)
        fun onStatus(status: String, pid: Int?, code: Int?)
        fun onError(message: String)
        fun onConnectionStateChanged(state: ConnectionState)
    }

    private val client = OkHttpClient.Builder()
        .connectTimeout(10, TimeUnit.SECONDS)
        .readTimeout(0, TimeUnit.SECONDS)  // SSE 串流不設讀取超時
        .writeTimeout(10, TimeUnit.SECONDS)
        .retryOnConnectionFailure(true)
        .build()

    private var listener: EventListener? = null
    private var job: Job? = null
    @Volatile
    private var isRunning = false

    fun setEventListener(l: EventListener) {
        listener = l
    }

    /** 啟動 SSE 連線（在指定的 CoroutineScope 中執行） */
    fun start(scope: CoroutineScope) {
        if (isRunning) return
        isRunning = true

        job = scope.launch(Dispatchers.IO) {
            var retryDelay = 1000L
            val maxRetryDelay = 30000L

            while (isActive && isRunning) {
                try {
                    listener?.onConnectionStateChanged(ConnectionState.CONNECTING)

                    // 步驟一：取得當前任務 ID
                    val taskId = getActiveTask()
                    if (taskId == null) {
                        listener?.onError("沒有進行中的翻譯任務")
                        listener?.onConnectionStateChanged(ConnectionState.RECONNECTING)
                        delay(retryDelay)
                        retryDelay = (retryDelay * 2).coerceAtMost(maxRetryDelay)
                        continue
                    }

                    // 步驟二：建立 SSE 連線
                    retryDelay = 1000L
                    connectToStream(taskId)

                    // 連線結束後嘗試重連
                    if (isRunning) {
                        listener?.onConnectionStateChanged(ConnectionState.RECONNECTING)
                        delay(retryDelay)
                    }
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    listener?.onError("連線錯誤: ${e.message}")
                    listener?.onConnectionStateChanged(ConnectionState.RECONNECTING)
                    delay(retryDelay)
                    retryDelay = (retryDelay * 2).coerceAtMost(maxRetryDelay)
                }
            }

            listener?.onConnectionStateChanged(ConnectionState.DISCONNECTED)
        }
    }

    /** 停止 SSE 連線 */
    fun stop() {
        isRunning = false
        job?.cancel()
        job = null
    }


    /** 取得當前活躍的翻譯任務 ID */
    private fun getActiveTask(): String? {
        val request = Request.Builder()
            .url("$baseUrl/api/translation/active-task")
            .get()
            .build()

        client.newCall(request).execute().use { response ->
            if (response.isSuccessful) {
                val json = JSONObject(response.body?.string() ?: "")
                if (json.optBoolean("success", false)) {
                    val taskId = json.optString("task_id", "")
                    return if (taskId.isNotEmpty() && taskId != "null") taskId else null
                }
            }
        }
        return null
    }

    /** 連線至 SSE 串流端點並持續讀取事件 */
    private suspend fun connectToStream(taskId: String) {
        val request = Request.Builder()
            .url("$baseUrl/api/translation/stream/$taskId")
            .header("Accept", "text/event-stream")
            .header("Cache-Control", "no-cache")
            .get()
            .build()

        val response = client.newCall(request).execute()
        if (!response.isSuccessful) {
            listener?.onError("SSE 連線失敗: HTTP ${response.code}")
            return
        }

        listener?.onConnectionStateChanged(ConnectionState.CONNECTED)

        val reader = BufferedReader(InputStreamReader(response.body?.byteStream() ?: return))
        var currentEvent = ""
        var currentData = ""

        try {
            while (isRunning && currentCoroutineContext().isActive) {
                val line = reader.readLine() ?: break

                when {
                    // 心跳包（SSE 註解以 : 開頭）
                    line.startsWith(":") -> { /* 忽略 */ }
                    // 事件類型
                    line.startsWith("event:") -> {
                        currentEvent = line.removePrefix("event:").trim()
                    }
                    // 事件資料
                    line.startsWith("data:") -> {
                        currentData = line.removePrefix("data:").trim()
                    }
                    // 空行 = 事件分隔符，處理累積的事件
                    line.isEmpty() && currentData.isNotEmpty() -> {
                        processEvent(currentEvent, currentData)
                        currentEvent = ""
                        currentData = ""
                    }
                }
            }
        } finally {
            reader.close()
            response.close()
        }
    }

    /** 處理解析後的 SSE 事件 */
    private fun processEvent(event: String, data: String) {
        try {
            when (event) {
                "subtitle" -> {
                    val json = JSONObject(data)
                    val entry = SubtitleEntry(
                        timestamp = json.optString("timestamp", ""),
                        original = json.optString("original", ""),
                        translated = json.optString("translated", "")
                    )
                    listener?.onSubtitle(entry)
                }
                "status" -> {
                    val json = JSONObject(data)
                    listener?.onStatus(
                        json.optString("status", "unknown"),
                        if (json.has("pid")) json.optInt("pid") else null,
                        if (json.has("code")) json.optInt("code") else null
                    )
                }
                "error" -> {
                    val json = JSONObject(data)
                    listener?.onError(json.optString("message", "未知錯誤"))
                }
                else -> {
                    // 嘗試作為字幕處理（向後相容）
                    try {
                        val json = JSONObject(data)
                        if (json.has("original") && json.has("translated")) {
                            val entry = SubtitleEntry(
                                timestamp = json.optString("timestamp", ""),
                                original = json.optString("original", ""),
                                translated = json.optString("translated", "")
                            )
                            listener?.onSubtitle(entry)
                        }
                    } catch (_: Exception) { /* 忽略無法解析的事件 */ }
                }
            }
        } catch (e: Exception) {
            listener?.onError("解析事件錯誤: ${e.message}")
        }
    }
}
