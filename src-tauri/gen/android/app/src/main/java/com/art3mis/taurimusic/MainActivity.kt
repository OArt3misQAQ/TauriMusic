package com.art3mis.taurimusic

import android.content.Intent
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.OpenableColumns
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import org.json.JSONObject
import java.io.File

class MainActivity : TauriActivity() {
  // 扫描器直放格式(与 Rust 端 scanner::EXTENSIONS 对齐):移动端没有 ffmpeg,
  // 其它格式分享进来也扫不到,提前挡掉免得用户以为"打开了却没反应"
  private val audioExts = setOf("mp3", "m4a", "flac", "ogg", "oga", "opus", "wav")

  private var safeTopDp = 0f
  private var safeBottomDp = 0f

  override fun onCreate(savedInstanceState: Bundle?) {
    // debug 包开启 WebView 远程调试,便于 chrome devtools 排查前端问题
    if (BuildConfig.DEBUG) {
      WebView.setWebContentsDebuggingEnabled(true)
    }
    enableEdgeToEdge()
    // 系统默认给透明状态栏/导航栏铺一层对比度 scrim,会把页面自己的深色背景
    // 盖成浅灰、图标颜色也跟着错判;页面已自行处理明暗,这里关掉
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
      window.isStatusBarContrastEnforced = false
      window.isNavigationBarContrastEnforced = false
    }
    super.onCreate(savedInstanceState)
    if (savedInstanceState == null) handleShareIntent(intent)
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    setIntent(intent)
    handleShareIntent(intent)
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    webView.addJavascriptInterface(SystemBarsBridge(), "TauriMusicBars")
    setupBackNavigation(webView)
    setupSafeArea(webView)
  }

  /** 状态栏/导航栏图标明暗跟随页面:播放页固定深色,其余跟随应用主题 */
  private inner class SystemBarsBridge {
    @JavascriptInterface
    fun setDark(dark: Boolean) {
      runOnUiThread {
        val controller = WindowCompat.getInsetsController(window, window.decorView)
        controller.isAppearanceLightStatusBars = !dark
        controller.isAppearanceLightNavigationBars = !dark
        // 三键导航栏不会跟着 WebView 内容走:按页面明暗铺底色,
        // 否则深色播放页底下会露出一条白栏
        window.navigationBarColor = if (dark) Color.BLACK else Color.WHITE
      }
    }

    /** 全屏播放页隐藏系统导航栏:三键导航的白底会在深色播放页下露出白条 */
    @JavascriptInterface
    fun setImmersive(on: Boolean) {
      runOnUiThread {
        val controller = WindowCompat.getInsetsController(window, window.decorView)
        if (on) {
          controller.systemBarsBehavior =
            WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
          controller.hide(WindowInsetsCompat.Type.navigationBars())
        } else {
          controller.show(WindowInsetsCompat.Type.navigationBars())
        }
      }
    }
  }

  /** 返回键先交给前端:关闭播放页/弹窗/抽屉,前端返回 false 才退出应用 */
  private fun setupBackNavigation(webView: WebView) {
    onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
      override fun handleOnBackPressed() {
        webView.evaluateJavascript("window.__tmBack ? window.__tmBack() : false") { result ->
          if (result != "true") {
            isEnabled = false
            onBackPressedDispatcher.onBackPressed()
            isEnabled = true
          }
        }
      }
    })
  }

  /**
   * edge-to-edge 下 WebView 会铺到状态栏/手势条底下:把系统栏高度注入成 CSS 变量,
   * 由前端给顶栏、播放页与底部操作区让位。Chrome 的 env(safe-area-inset-*)
   * 在 Android WebView 里只覆盖刘海,不覆盖状态栏与手势条,不能直接用
   */
  private fun setupSafeArea(webView: WebView) {
    ViewCompat.setOnApplyWindowInsetsListener(webView) { view, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
      )
      val density = resources.displayMetrics.density
      val top = bars.top / density
      val bottom = bars.bottom / density
      if (top != safeTopDp || bottom != safeBottomDp) {
        safeTopDp = top
        safeBottomDp = bottom
        injectSafeArea(view as WebView)
      }
      insets
    }
    ViewCompat.requestApplyInsets(webView)
    // 首次布局时页面还没加载完,注入会丢;等首屏渲染后再补两次
    webView.postDelayed({ injectSafeArea(webView) }, 1000)
    webView.postDelayed({ injectSafeArea(webView) }, 2500)
  }

  private fun injectSafeArea(webView: WebView) {
    webView.evaluateJavascript(
      "document.documentElement.style.setProperty('--tm-safe-top','${safeTopDp}px');" +
        "document.documentElement.style.setProperty('--tm-safe-bottom','${safeBottomDp}px');",
      null,
    )
  }

  /** 系统分享 / 用 TauriMusic 打开音频:复制到缓存收件箱,Rust 侧消费后走导入管线 */
  private fun handleShareIntent(intent: Intent?) {
    if (intent == null) return
    val uris = when (intent.action) {
      Intent.ACTION_SEND -> listOfNotNull(streamUri(intent))
      Intent.ACTION_SEND_MULTIPLE -> streamUris(intent)
      Intent.ACTION_VIEW -> listOfNotNull(intent.data)
      else -> return
    }
    if (uris.isEmpty()) return
    // 大文件复制不能卡主线程;前端轮询收件箱,复制完成自然能取到
    Thread {
      for (uri in uris) {
        runCatching { copySharedUri(uri) }
      }
    }.start()
  }

  @Suppress("DEPRECATION")
  private fun streamUri(intent: Intent): Uri? =
    intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM)
      ?: intent.clipData?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.uri

  @Suppress("DEPRECATION")
  private fun streamUris(intent: Intent): List<Uri> {
    val out = ArrayList<Uri>()
    intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)?.let(out::addAll)
    intent.clipData?.let { clip ->
      for (i in 0 until clip.itemCount) out.add(clip.getItemAt(i).uri)
    }
    return out
  }

  private fun copySharedUri(uri: Uri) {
    val mime = contentResolver.getType(uri) ?: ""
    var name = queryDisplayName(uri)
    if (name.isNullOrBlank() || !name.contains('.')) {
      val ext = extFromMime(mime)
      if (ext != null) name = "${name?.takeIf { it.isNotBlank() } ?: "shared"}.$ext"
    }
    if (!name.isNullOrBlank() && !hasAudioExt(name)) return

    val safeName = sanitize(name ?: "shared.$mime")
    val inbox = File(cacheDir, "shared-inbox").apply { mkdirs() }
    // 保留原文件名:无标签文件的歌名由文件名回退解析,加时间戳前缀会污染网易云匹配
    val dest = uniqueDest(inbox, safeName)
    try {
      contentResolver.openInputStream(uri)?.use { input ->
        dest.outputStream().use { output -> input.copyTo(output) }
      } ?: return
    } catch (_: Exception) {
      dest.delete()
      return
    }
    if (dest.length() == 0L) {
      dest.delete()
      return
    }
    writeSidecar(inbox, dest, safeName)
  }

  /** 同名文件加序号(保留扩展名),createNewFile 原子占位防并发分享互相覆盖 */
  private fun uniqueDest(dir: File, name: String): File {
    val dot = name.lastIndexOf('.')
    val base = if (dot > 0) name.substring(0, dot) else name
    val ext = if (dot > 0) name.substring(dot) else ""
    var dest = File(dir, name)
    var i = 1
    while (!dest.createNewFile()) {
      dest = File(dir, "$base ($i)$ext")
      i++
    }
    return dest
  }

  /** 收件箱说明文件:一音频一 JSON,Rust 读一份删一份,两端无需共享队列锁 */
  private fun writeSidecar(inbox: File, dest: File, name: String) {
    val tmp = File(inbox, "${dest.name}.json.tmp")
    val sidecar = File(inbox, "${dest.name}.json")
    try {
      tmp.writeText(JSONObject().put("path", dest.absolutePath).put("name", name).toString())
      if (!tmp.renameTo(sidecar)) throw IllegalStateException("sidecar rename failed")
    } catch (_: Exception) {
      tmp.delete()
      dest.delete()
    }
  }

  private fun queryDisplayName(uri: Uri): String? {
    if (uri.scheme == "file") return uri.lastPathSegment
    return try {
      contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
        if (c.moveToFirst()) c.getString(0) else null
      }
    } catch (_: Exception) {
      null
    }
  }

  private fun extFromMime(mime: String): String? = when {
    mime.contains("mpeg") -> "mp3"
    mime.contains("flac") -> "flac"
    mime.contains("wav") -> "wav"
    mime.contains("opus") || mime.contains("ogg") -> "ogg"
    mime.contains("mp4") || mime.contains("m4a") -> "m4a"
    else -> null
  }

  private fun hasAudioExt(name: String): Boolean =
    name.substringAfterLast('.', "").lowercase() in audioExts

  private fun sanitize(name: String): String =
    name.map { if (it in "\\/:*?\"<>|" || it.isISOControl()) '_' else it }
      .joinToString("")
      .trim()
      .takeLast(120)
      .ifBlank { "shared" }
}
