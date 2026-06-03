package com.translator.subtitleoverlay

import android.content.Context
import android.content.SharedPreferences
import android.graphics.Color

/**
 * 設定管理器 — 封裝 SharedPreferences，持久化所有使用者設定
 */
class SettingsManager(context: Context) {

    companion object {
        const val DEFAULT_FONT_FAMILY = "sans-serif"

        fun normalizeFontFamily(value: String?): String {
            val normalized = value.orEmpty().trim().replace(Regex("\\s+"), " ")
            return normalized.ifEmpty { DEFAULT_FONT_FAMILY }.take(120)
        }
    }

    private val prefs: SharedPreferences =
        context.getSharedPreferences("subtitle_overlay_settings", Context.MODE_PRIVATE)

    // === 連線設定 ===

    /** 伺服器完整 URL（例如 http://192.168.1.100:8765） */
    var serverUrl: String
        get() = prefs.getString("server_url", "http://192.168.1.100:8765") ?: "http://192.168.1.100:8765"
        set(value) = prefs.edit().putString("server_url", value).apply()

    // === 外觀設定 ===

    /** 原文文字顏色（預設：淺藍色） */
    var originalTextColor: Int
        get() = prefs.getInt("original_text_color", Color.parseColor("#81D4FA"))
        set(value) = prefs.edit().putInt("original_text_color", value).apply()

    /** 翻譯文字顏色（預設：黃色） */
    var translatedTextColor: Int
        get() = prefs.getInt("translated_text_color", Color.parseColor("#FFD54F"))
        set(value) = prefs.edit().putInt("translated_text_color", value).apply()

    /** 背景顏色（預設：黑色） */
    var backgroundColor: Int
        get() = prefs.getInt("background_color", Color.parseColor("#000000"))
        set(value) = prefs.edit().putInt("background_color", value).apply()

    /** 背景透明度 0-100（預設：70%） */
    var backgroundOpacity: Int
        get() = prefs.getInt("background_opacity", 70)
        set(value) = prefs.edit().putInt("background_opacity", value).apply()

    /** 字體大小 sp（預設：16） */
    var fontSize: Float
        get() = prefs.getFloat("font_size", 16f)
        set(value) = prefs.edit().putFloat("font_size", value).apply()

    /** 字體族（Android 內建 family name，預設：sans-serif） */
    var fontFamily: String
        get() = normalizeFontFamily(prefs.getString("font_family", DEFAULT_FONT_FAMILY))
        set(value) = prefs.edit().putString("font_family", normalizeFontFamily(value)).apply()

    /** 最大字幕行數（預設：3） */
    var maxSubtitleCount: Int
        get() = prefs.getInt("max_subtitle_count", 3)
        set(value) = prefs.edit().putInt("max_subtitle_count", value).apply()

    /** 浮窗寬度（px，0 = 螢幕 80%） */
    var overlayWidth: Int
        get() = prefs.getInt("overlay_width", 0)
        set(value) = prefs.edit().putInt("overlay_width", value).apply()

    /** 浮窗高度（px，0 = 200dp） */
    var overlayHeight: Int
        get() = prefs.getInt("overlay_height", 0)
        set(value) = prefs.edit().putInt("overlay_height", value).apply()

    /**
     * 取得含透明度的背景色（將 opacity 百分比轉換為 alpha 通道）
     */
    fun getBackgroundColorWithAlpha(): Int {
        val alpha = (backgroundOpacity * 255 / 100)
        val rgb = backgroundColor and 0x00FFFFFF
        return (alpha shl 24) or rgb
    }
}
