package com.art3mis.taurimusic

import android.os.Bundle
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    // debug 包开启 WebView 远程调试,便于 chrome devtools 排查前端问题
    if (BuildConfig.DEBUG) {
      WebView.setWebContentsDebuggingEnabled(true)
    }
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }
}
