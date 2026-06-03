package com.translator.subtitleoverlay

import android.app.AlertDialog
import android.content.Intent
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.EditText
import android.widget.GridLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Spinner
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import com.google.android.material.button.MaterialButton
import com.google.android.material.slider.Slider

/**
 * 主畫面 — 連線設定、字幕外觀設定、預覽、啟動/停止控制
 */
class MainActivity : AppCompatActivity() {

    private data class FontFamilyOption(val label: String, val family: String)

    private val fontFamilyOptions = listOf(
        FontFamilyOption("系統預設 Sans", "sans-serif"),
        FontFamilyOption("Serif", "serif"),
        FontFamilyOption("Monospace", "monospace"),
        FontFamilyOption("Sans Medium", "sans-serif-medium"),
        FontFamilyOption("Sans Condensed", "sans-serif-condensed"),
        FontFamilyOption("Casual", "casual"),
        FontFamilyOption("Cursive", "cursive")
    )

    private lateinit var settings: SettingsManager
    private var isServiceRunning = false

    // UI 元件
    private lateinit var editServerUrl: EditText
    private lateinit var statusIndicator: View
    private lateinit var textStatus: TextView
    private lateinit var colorOriginal: View
    private lateinit var colorTranslated: View
    private lateinit var colorBackground: View
    private lateinit var sliderOpacity: Slider
    private lateinit var textOpacity: TextView
    private lateinit var sliderFontSize: Slider
    private lateinit var textFontSize: TextView
    private lateinit var spinnerFontFamily: Spinner
    private lateinit var sliderSubtitleCount: Slider
    private lateinit var textSubtitleCount: TextView
    private lateinit var previewContainer: LinearLayout
    private lateinit var btnToggle: MaterialButton
    private lateinit var logTextView: TextView
    private lateinit var logScrollView: ScrollView
    private lateinit var btnClearLog: MaterialButton
    private val logBuffer = StringBuilder()
    private val maxLogLines = 200

    // 預設色板
    private val presetColors = intArrayOf(
        Color.parseColor("#FFFFFF"), Color.parseColor("#FFD54F"),
        Color.parseColor("#81D4FA"), Color.parseColor("#69F0AE"),
        Color.parseColor("#FF80AB"), Color.parseColor("#FFAB40"),
        Color.parseColor("#CE93D8"), Color.parseColor("#F44336"),
        Color.parseColor("#4CAF50"), Color.parseColor("#2196F3"),
        Color.parseColor("#FF9800"), Color.parseColor("#00BCD4"),
        Color.parseColor("#E0E0E0"), Color.parseColor("#9E9E9E"),
        Color.parseColor("#616161"), Color.parseColor("#212121"),
        Color.parseColor("#000000"), Color.parseColor("#1B5E20")
    )

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        settings = SettingsManager(this)
        bindViews()
        loadSettings()
        setupListeners()
        updatePreview()
    }

    private fun bindViews() {
        editServerUrl = findViewById(R.id.edit_server_url)
        statusIndicator = findViewById(R.id.status_indicator)
        textStatus = findViewById(R.id.text_status)
        colorOriginal = findViewById(R.id.color_original)
        colorTranslated = findViewById(R.id.color_translated)
        colorBackground = findViewById(R.id.color_background)
        sliderOpacity = findViewById(R.id.slider_opacity)
        textOpacity = findViewById(R.id.text_opacity)
        sliderFontSize = findViewById(R.id.slider_font_size)
        textFontSize = findViewById(R.id.text_font_size)
        spinnerFontFamily = findViewById(R.id.spinner_font_family)
        sliderSubtitleCount = findViewById(R.id.slider_subtitle_count)
        textSubtitleCount = findViewById(R.id.text_subtitle_count)
        previewContainer = findViewById(R.id.preview_container)
        btnToggle = findViewById(R.id.btn_toggle)
        logTextView = findViewById(R.id.text_log)
        logScrollView = findViewById(R.id.scroll_log)
        btnClearLog = findViewById(R.id.btn_clear_log)
    }

    private fun loadSettings() {
        editServerUrl.setText(settings.serverUrl)

        setColorViewBackground(colorOriginal, settings.originalTextColor)
        setColorViewBackground(colorTranslated, settings.translatedTextColor)
        setColorViewBackground(colorBackground, settings.backgroundColor)

        sliderOpacity.value = settings.backgroundOpacity.toFloat()
        textOpacity.text = "${settings.backgroundOpacity}%"

        sliderFontSize.value = settings.fontSize
        textFontSize.text = "${settings.fontSize.toInt()}sp"

        setupFontFamilySpinner()

        sliderSubtitleCount.value = settings.maxSubtitleCount.toFloat()
        textSubtitleCount.text = "${settings.maxSubtitleCount} 組"
    }

    private fun setupListeners() {
        // 顏色選擇器
        colorOriginal.setOnClickListener {
            showColorPicker(settings.originalTextColor) { color ->
                settings.originalTextColor = color
                setColorViewBackground(colorOriginal, color)
                refreshAppearance()
            }
        }

        colorTranslated.setOnClickListener {
            showColorPicker(settings.translatedTextColor) { color ->
                settings.translatedTextColor = color
                setColorViewBackground(colorTranslated, color)
                refreshAppearance()
            }
        }

        colorBackground.setOnClickListener {
            showColorPicker(settings.backgroundColor) { color ->
                settings.backgroundColor = color
                setColorViewBackground(colorBackground, color)
                refreshAppearance()
            }
        }

        // 透明度滑桿
        sliderOpacity.addOnChangeListener { _, value, _ ->
            settings.backgroundOpacity = value.toInt()
            textOpacity.text = "${value.toInt()}%"
            refreshAppearance()
        }

        // 字體大小滑桿
        sliderFontSize.addOnChangeListener { _, value, _ ->
            settings.fontSize = value
            textFontSize.text = "${value.toInt()}sp"
            refreshAppearance()
        }

        // 字體選擇
        spinnerFontFamily.onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
            override fun onItemSelected(
                parent: AdapterView<*>?,
                view: View?,
                position: Int,
                id: Long
            ) {
                settings.fontFamily = fontFamilyOptions.getOrNull(position)?.family
                    ?: SettingsManager.DEFAULT_FONT_FAMILY
                refreshAppearance()
            }

            override fun onNothingSelected(parent: AdapterView<*>?) = Unit
        }

        // 字幕行數滑桿
        sliderSubtitleCount.addOnChangeListener { _, value, _ ->
            settings.maxSubtitleCount = value.toInt()
            textSubtitleCount.text = "${value.toInt()} 組"
            refreshAppearance()
        }

        // 啟動/停止按鈕
        btnToggle.setOnClickListener {
            if (isServiceRunning) {
                stopOverlayService()
            } else {
                saveConnectionSettings()
                checkPermissionAndStart()
            }
        }

        // 清除 Log 按鈕
        btnClearLog.setOnClickListener {
            logBuffer.clear()
            logTextView.text = ""
        }
    }

    private fun setupFontFamilySpinner() {
        val adapter = ArrayAdapter(
            this,
            R.layout.item_font_spinner,
            fontFamilyOptions.map { it.label }
        )
        adapter.setDropDownViewResource(R.layout.item_font_spinner_dropdown)
        spinnerFontFamily.adapter = adapter

        val selectedFamily = SettingsManager.normalizeFontFamily(settings.fontFamily)
        val selectedIndex = fontFamilyOptions.indexOfFirst { it.family == selectedFamily }
            .takeIf { it >= 0 }
            ?: fontFamilyOptions.indexOfFirst { it.family == SettingsManager.DEFAULT_FONT_FAMILY }
                .coerceAtLeast(0)
        spinnerFontFamily.setSelection(selectedIndex, false)
    }

    // === 顏色選擇器 ===

    private fun showColorPicker(currentColor: Int, onColorSelected: (Int) -> Unit) {
        val dialogView = layoutInflater.inflate(R.layout.dialog_color_picker, null)
        val colorGrid = dialogView.findViewById<GridLayout>(R.id.color_grid)
        val editHex = dialogView.findViewById<EditText>(R.id.edit_hex_color)
        val previewLarge = dialogView.findViewById<View>(R.id.color_preview_large)

        // 設定目前色碼
        editHex.setText(String.format("#%06X", currentColor and 0xFFFFFF))
        previewLarge.setBackgroundColor(currentColor)

        var selectedColor = currentColor

        // 填充預設色盤
        val size = (44 * resources.displayMetrics.density).toInt()
        val margin = (4 * resources.displayMetrics.density).toInt()

        for (color in presetColors) {
            val colorView = View(this).apply {
                val params = GridLayout.LayoutParams().apply {
                    width = size
                    height = size
                    setMargins(margin, margin, margin, margin)
                }
                layoutParams = params

                val drawable = GradientDrawable().apply {
                    shape = GradientDrawable.RECTANGLE
                    cornerRadius = 8 * resources.displayMetrics.density
                    setColor(color)
                    setStroke(
                        if (color == currentColor) 3 else 1,
                        if (color == currentColor) Color.WHITE else Color.GRAY
                    )
                }
                background = drawable

                setOnClickListener {
                    selectedColor = color
                    editHex.setText(String.format("#%06X", color and 0xFFFFFF))
                    previewLarge.setBackgroundColor(color)
                }
            }
            colorGrid.addView(colorView)
        }

        val dialog = AlertDialog.Builder(this)
            .setTitle("選擇顏色")
            .setView(dialogView)
            .setPositiveButton("確認") { _, _ ->
                // 嘗試從 Hex 輸入取得顏色
                val hexText = editHex.text.toString().trim()
                val finalColor = try {
                    Color.parseColor(if (hexText.startsWith("#")) hexText else "#$hexText")
                } catch (_: Exception) {
                    selectedColor
                }
                onColorSelected(finalColor)
            }
            .setNegativeButton("取消", null)
            .create()

        dialog.show()

        // 強制覆蓋對話框背景和文字顏色（繞過主題系統）
        dialog.window?.setBackgroundDrawableResource(android.R.color.transparent)
        dialog.window?.decorView?.setBackgroundColor(Color.parseColor("#1E1E2E"))
        dialog.getButton(AlertDialog.BUTTON_POSITIVE)?.setTextColor(Color.parseColor("#BB86FC"))
        dialog.getButton(AlertDialog.BUTTON_NEGATIVE)?.setTextColor(Color.parseColor("#BB86FC"))
        // 標題文字
        val titleId = resources.getIdentifier("alertTitle", "id", "android")
        if (titleId > 0) {
            dialog.findViewById<TextView>(titleId)?.setTextColor(Color.WHITE)
        }
    }

    // === 預覽 ===

    private fun updatePreview() {
        previewContainer.removeAllViews()
        previewContainer.setBackgroundColor(settings.getBackgroundColorWithAlpha())

        // 模擬字幕資料
        val sampleSubtitles = listOf(
            SubtitleEntry("", "こんにちは、皆さん", "大家好"),
            SubtitleEntry("", "今日はいい天気ですね", "今天天氣真好呢"),
            SubtitleEntry("", "よろしくお願いします", "請多多指教")
        )

        val count = settings.maxSubtitleCount.coerceAtMost(sampleSubtitles.size)
        for (i in 0 until count) {
            val entry = sampleSubtitles[i]
            val entryLayout = LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(12, 4, 12, 4)
            }

            val originalView = TextView(this).apply {
                text = entry.original
                setTextColor(settings.originalTextColor)
                setTextSize(TypedValue.COMPLEX_UNIT_SP, settings.fontSize)
                typeface = subtitleTypeface(Typeface.NORMAL)
                setShadowLayer(2f, 1f, 1f, Color.BLACK)
            }
            entryLayout.addView(originalView)

            val translatedView = TextView(this).apply {
                text = entry.translated
                setTextColor(settings.translatedTextColor)
                setTextSize(TypedValue.COMPLEX_UNIT_SP, settings.fontSize)
                typeface = subtitleTypeface(Typeface.BOLD)
                setShadowLayer(2f, 1f, 1f, Color.BLACK)
            }
            entryLayout.addView(translatedView)

            previewContainer.addView(entryLayout)

            // 分隔線
            if (i < count - 1) {
                val divider = View(this).apply {
                    layoutParams = LinearLayout.LayoutParams(
                        LinearLayout.LayoutParams.MATCH_PARENT, 1
                    ).apply { setMargins(12, 2, 12, 2) }
                    setBackgroundColor(Color.argb(60, 255, 255, 255))
                }
                previewContainer.addView(divider)
            }
        }
    }

    private fun refreshAppearance() {
        updatePreview()
        OverlayService.instance?.refreshAppearance()
    }

    private fun subtitleTypeface(style: Int): Typeface {
        return Typeface.create(settings.fontFamily, style)
    }

    // === 服務控制 ===

    private fun saveConnectionSettings() {
        val url = editServerUrl.text.toString().trim()
        // 若使用者沒有輸入協定，自動補上 http://
        settings.serverUrl = if (url.startsWith("http://") || url.startsWith("https://")) {
            url
        } else {
            "http://$url"
        }
    }

    private fun checkPermissionAndStart() {
        // 檢查懸浮窗權限
        if (!Settings.canDrawOverlays(this)) {
            Toast.makeText(this, "請授予懸浮窗權限", Toast.LENGTH_LONG).show()
            val intent = Intent(
                Settings.ACTION_MANAGE_OVERLAY_PERMISSION,
                Uri.parse("package:$packageName")
            )
            startActivity(intent)
            return
        }

        // 檢查通知權限（Android 13+）
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            if (checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS)
                != android.content.pm.PackageManager.PERMISSION_GRANTED
            ) {
                requestPermissions(
                    arrayOf(android.Manifest.permission.POST_NOTIFICATIONS),
                    100
                )
                return
            }
        }

        startOverlayService()
    }

    override fun onRequestPermissionsResult(
        requestCode: Int, permissions: Array<out String>, grantResults: IntArray
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == 100) {
            // 即使通知權限未授予也啟動服務
            startOverlayService()
        }
    }

    private fun startOverlayService() {
        val intent = Intent(this, OverlayService::class.java).apply {
            putExtra(OverlayService.EXTRA_SERVER_URL, settings.serverUrl)
        }
        startForegroundService(intent)
        isServiceRunning = true
        updateToggleButton()

        // 註冊 Log 回呼
        OverlayService.instance?.logCallback = object : OverlayService.LogCallback {
            override fun onLog(message: String) {
                runOnUiThread { appendLog(message) }
            }
        }
        appendLog("服務啟動請求已發送")
    }

    private fun stopOverlayService() {
        val intent = Intent(this, OverlayService::class.java).apply {
            action = OverlayService.ACTION_STOP
        }
        startService(intent)
        isServiceRunning = false
        updateToggleButton()
    }

    private fun updateToggleButton() {
        if (isServiceRunning) {
            btnToggle.text = "停止浮窗字幕"
            btnToggle.setBackgroundColor(Color.parseColor("#F44336"))
            setStatusIndicator(Color.parseColor("#4CAF50"), "服務運行中")
        } else {
            btnToggle.text = "啟動浮窗字幕"
            btnToggle.setBackgroundColor(Color.parseColor("#6C63FF"))
            setStatusIndicator(Color.parseColor("#F44336"), "未連線")
        }
    }

    // === 工具方法 ===

    private fun setColorViewBackground(view: View, color: Int) {
        val drawable = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = 8 * resources.displayMetrics.density
            setColor(color)
            setStroke(1, Color.GRAY)
        }
        view.background = drawable
    }

    private fun setStatusIndicator(color: Int, text: String) {
        val drawable = GradientDrawable().apply {
            shape = GradientDrawable.OVAL
            setColor(color)
        }
        statusIndicator.background = drawable
        textStatus.text = text
    }

    // === Log 管理 ===

    private fun appendLog(message: String) {
        logBuffer.appendLine(message)

        // 限制行數
        val lines = logBuffer.lines()
        if (lines.size > maxLogLines) {
            logBuffer.clear()
            logBuffer.append(lines.takeLast(maxLogLines).joinToString("\n"))
        }

        logTextView.text = logBuffer.toString()
        logScrollView.post {
            logScrollView.fullScroll(View.FOCUS_DOWN)
        }
    }

    override fun onResume() {
        super.onResume()
        // 重新連接 Log 回呼
        OverlayService.instance?.logCallback = object : OverlayService.LogCallback {
            override fun onLog(message: String) {
                runOnUiThread { appendLog(message) }
            }
        }
    }

    override fun onPause() {
        super.onPause()
        OverlayService.instance?.logCallback = null
    }
}
