package xyz.block.buzz.mobile

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Color
import android.view.View
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodChannel
import io.flutter.plugin.common.StandardMessageCodec
import io.flutter.plugin.platform.PlatformView
import io.flutter.plugin.platform.PlatformViewFactory
import java.io.ByteArrayInputStream

// Only bundled avatar assets may enter the local HTML document.
internal fun isTransparentAvatarVideo(url: String): Boolean = Regex(
    "https://dwwgwmfqqjotj\\.cloudfront\\.net/avatars/[A-Za-z0-9_-]+/webm/[A-Za-z0-9_-]+/[A-Za-z0-9_-]+\\.webm",
).matches(url)

internal class TransparentAvatarViewFactory(private val messenger: BinaryMessenger) :
    PlatformViewFactory(StandardMessageCodec.INSTANCE) {
    override fun create(context: Context, viewId: Int, args: Any?): PlatformView {
        val parameters = args as? Map<*, *>
        return TransparentAvatarView(context, viewId, messenger, parameters?.get("url") as? String ?: "")
    }
}

/** Chromium composites WebM alpha; Android's native video decoder drops it. */
@SuppressLint("SetJavaScriptEnabled")
private class TransparentAvatarView(
    context: Context,
    id: Int,
    messenger: BinaryMessenger,
    url: String,
) : PlatformView {
    private val webView = WebView(context)
    private val channel = MethodChannel(messenger, "buzz/transparent-avatar/$id")
    private var playing = false
    private var disposed = false

    init {
        webView.setBackgroundColor(Color.TRANSPARENT)
        webView.isFocusable = false
        webView.isClickable = false
        webView.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
        webView.settings.apply {
            javaScriptEnabled = true
            mediaPlaybackRequiresUserGesture = false
            allowFileAccess = false
            allowContentAccess = false
            domStorageEnabled = false
            mixedContentMode = android.webkit.WebSettings.MIXED_CONTENT_NEVER_ALLOW
        }
        val valid = isTransparentAvatarVideo(url)
        val poster = url.replace("/webm/", "/poster/").removeSuffix(".webm") + ".png"
        webView.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest) = true
            override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? {
                val target = request.url.toString()
                return if (valid && (target == url || target == poster)) null else
                    WebResourceResponse("text/plain", "UTF-8", ByteArrayInputStream(ByteArray(0)))
            }
            override fun onPageFinished(view: WebView, loadedUrl: String) { updatePlayback() }
        }
        channel.setMethodCallHandler { call, result ->
            if (!valid) {
                result.error("unsupported_avatar", "The avatar is not a bundled HTTPS WebM.", null)
            } else if (call.method == "setPlaying") {
                playing = call.arguments == true
                updatePlayback()
                result.success(null)
            } else {
                result.notImplemented()
            }
        }
        if (valid) {
            // No remote HTML, scripts, navigation, native JS bridge, or user-provided markup.
            webView.loadDataWithBaseURL(
                "https://dwwgwmfqqjotj.cloudfront.net/",
                """<!doctype html><html><head>
                <meta name="viewport" content="width=device-width,initial-scale=1,maximum-scale=1">
                <meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src https://dwwgwmfqqjotj.cloudfront.net; media-src https://dwwgwmfqqjotj.cloudfront.net; style-src 'unsafe-inline'; script-src 'unsafe-inline'">
                <style>html,body{margin:0;width:100%;height:100%;overflow:hidden;background:transparent}video{width:100%;height:100%;object-fit:contain;pointer-events:none}</style>
                </head><body><video id="avatar" muted loop playsinline preload="auto" poster="$poster" src="$url"></video></body></html>""",
                "text/html", "UTF-8", null,
            )
        }
    }

    private fun updatePlayback() {
        if (disposed) return
        webView.evaluateJavascript(
            "var v=document.getElementById('avatar');if(v){" +
                if (playing) "v.play().catch(function(){});}" else "v.pause();}",
            null,
        )
    }

    override fun getView(): View = webView

    override fun dispose() {
        disposed = true
        channel.setMethodCallHandler(null)
        webView.stopLoading()
        webView.destroy()
    }
}
