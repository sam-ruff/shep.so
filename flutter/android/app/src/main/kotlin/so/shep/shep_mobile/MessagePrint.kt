package so.shep.shep_mobile

import android.app.Activity
import android.content.Context
import android.os.Bundle
import android.os.CancellationSignal
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.print.PageRange
import android.print.PrintAttributes
import android.print.PrintDocumentAdapter
import android.print.PrintManager
import android.webkit.JavascriptInterface
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import org.json.JSONObject
import java.io.ByteArrayInputStream

/** An owned, resource-confined WebView retained until Android finishes its PDF
 * adapter. Opening the system dialog is never reported as a print receipt. */
class MessagePrint(private val activity: Activity) {
    private var view: WebView? = null
    private var pending: MethodChannel.Result? = null
    private var generation: String? = null
    private var title = "Shep message"
    private val main = Handler(Looper.getMainLooper())
    private val deadline = Runnable { fail("The print document did not finish preparing. Try plain text or retry.") }

    fun handle(call: MethodCall, result: MethodChannel.Result) {
        if (call.method != "print") { result.notImplemented(); return }
        if (view != null) { result.error("busy", "Finish the current print dialog first.", null); return }
        val html = call.argument<String>("document")
        val token = call.argument<String>("generation")
        if (html.isNullOrEmpty() || html.length > 96 * 1024 * 1024 || token.isNullOrEmpty() || token.length > 128) {
            result.error("invalid", "Reopen this message and retry printing.", null); return
        }
        pending = result
        generation = token
        title = call.argument<String>("title")?.replace(Regex("[\\p{Cntrl}]"), " ")?.take(200)?.ifBlank { "Shep message" } ?: "Shep message"
        try {
            val web = WebView(activity)
            view = web
            web.settings.apply {
                javaScriptEnabled = true // Only the fixed CSP-hashed print runtime.
                allowFileAccess = false
                allowContentAccess = false
                blockNetworkLoads = true
                domStorageEnabled = false
                javaScriptCanOpenWindowsAutomatically = false
                setSupportMultipleWindows(false)
            }
            web.addJavascriptInterface(Bridge(), "ShepPrint")
            web.webViewClient = object : WebViewClient() {
                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest) = true
                override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? {
                    // WebView handles blob resources internally. Refuse every
                    // external/file/content request even if the CSP is changed.
                    return if (request.url.scheme in listOf("blob", "about", "data")) null
                    else WebResourceResponse("text/plain", "utf-8", ByteArrayInputStream(ByteArray(0)))
                }
                override fun onRenderProcessGone(view: WebView, detail: android.webkit.RenderProcessGoneDetail): Boolean {
                    fail("The print renderer stopped. Try plain text or retry."); return true
                }
            }
            main.postDelayed(deadline, 30_000)
            web.loadDataWithBaseURL(null, html, "text/html", "UTF-8", null)
        } catch (_: Exception) { fail("Could not open the print document. Retry.") }
    }

    private inner class Bridge {
        @JavascriptInterface fun postMessage(value: String) {
            if (value.length > 512) return
            val message = try { JSONObject(value) } catch (_: Exception) { return }
            main.post {
                if (message.optString("generation") != generation || pending == null) return@post
                when (message.optString("type")) {
                    "ready" -> openDialog()
                    "error" -> fail("Could not format the print document. Try plain text or retry.")
                }
            }
        }
    }

    private fun openDialog() {
        val web = view ?: return
        val result = pending ?: return
        main.removeCallbacks(deadline)
        try {
            val manager = activity.getSystemService(Context.PRINT_SERVICE) as? PrintManager
                ?: throw IllegalStateException("Unavailable")
            val delegate = web.createPrintDocumentAdapter(title)
            val adapter = object : PrintDocumentAdapter() {
                override fun onStart() = delegate.onStart()
                override fun onLayout(old: PrintAttributes?, attributes: PrintAttributes, cancel: CancellationSignal, callback: LayoutResultCallback, extras: Bundle?) =
                    delegate.onLayout(old, attributes, cancel, callback, extras)
                override fun onWrite(pages: Array<out PageRange>, destination: ParcelFileDescriptor, cancel: CancellationSignal, callback: WriteResultCallback) =
                    delegate.onWrite(pages, destination, cancel, callback)
                override fun onFinish() {
                    delegate.onFinish()
                    if (view === web) cleanup()
                }
            }
            manager.print(title, adapter, PrintAttributes.Builder().build())
            pending = null
            result.success(null) // The dialog was opened; cancellation belongs to Android.
        } catch (_: Exception) { fail("Could not open Android printing. Enable a print service or retry.") }
    }
    private fun cleanup() {
        main.removeCallbacks(deadline)
        generation = null
        view?.apply { stopLoading(); removeJavascriptInterface("ShepPrint"); destroy() }
        view = null
    }
    private fun fail(message: String) {
        val result = pending
        pending = null
        cleanup()
        result?.error("print", message, null)
    }
    fun dispose() = fail("Printing was interrupted when Shep closed. Reopen the message and retry.")
}
