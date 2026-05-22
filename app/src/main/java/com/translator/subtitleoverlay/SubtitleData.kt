package com.translator.subtitleoverlay

/**
 * 字幕資料模型
 */
data class SubtitleEntry(
    val timestamp: String,
    val original: String,
    val translated: String
)

/**
 * SSE 事件類型
 */
sealed class SseEvent {
    data class Subtitle(val entry: SubtitleEntry) : SseEvent()
    data class Status(val status: String, val pid: Int? = null, val code: Int? = null) : SseEvent()
    data class Error(val message: String) : SseEvent()
    object Ping : SseEvent()
}

/**
 * 連線狀態
 */
enum class ConnectionState {
    DISCONNECTED,
    CONNECTING,
    CONNECTED,
    RECONNECTING,
    ERROR
}
