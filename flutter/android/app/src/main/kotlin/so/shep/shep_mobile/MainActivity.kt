package so.shep.shep_mobile

import android.app.Activity
import android.content.Intent
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.util.concurrent.Executors

class MainActivity : FlutterActivity() {
    private var messagePrinter: MessagePrint? = null
    private var pending: MethodChannel.Result? = null
    private var content: ByteArray? = null
    private val writer = Executors.newSingleThreadExecutor()

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        messagePrinter = MessagePrint(this).also { printer ->
            MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "so.shep/message-print").setMethodCallHandler(printer::handle)
        }
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "so.shep/attachment-save")
            .setMethodCallHandler { call, result ->
                if (call.method != "save") { result.notImplemented(); return@setMethodCallHandler }
                if (pending != null) { result.error("busy", "Finish the current save first.", null); return@setMethodCallHandler }
                val bytes = call.argument<ByteArray>("bytes")
                val name = call.argument<String>("name")
                if (bytes == null || bytes.size > 25 * 1024 * 1024 || name.isNullOrBlank() || name.contains('/') || name.contains('\\')) {
                    result.error("invalid", "Reopen the attachment and retry.", null); return@setMethodCallHandler
                }
                pending = result; content = bytes
                try {
                    val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
                        addCategory(Intent.CATEGORY_OPENABLE)
                        type = call.argument<String>("type")?.takeIf { it.matches(Regex("[a-zA-Z0-9!#$&^_.+-]+/[a-zA-Z0-9!#$&^_.+-]+")) } ?: "application/octet-stream"
                        putExtra(Intent.EXTRA_TITLE, name)
                    }
                    @Suppress("DEPRECATION")
                    startActivityForResult(intent, SAVE_ATTACHMENT)
                } catch (_: Exception) {
                    pending = null; content = null
                    result.error("picker", "Could not open the save picker. Retry.", null)
                }
            }
    }

    @Deprecated("Android activity result callback required by FlutterActivity")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != SAVE_ATTACHMENT) return
        val result = pending ?: return
        val bytes = content
        val uri = data?.data
        if (resultCode != Activity.RESULT_OK || uri == null || bytes == null) {
            pending = null; content = null; result.success(false); return
        }
        // The picker grants exactly this document URI; stream writes never run
        // on the UI thread, and success follows closing the destination stream.
        writer.execute {
            val saved = try {
                contentResolver.openOutputStream(uri, "wt")?.use { it.write(bytes); it.flush() }
                    ?: throw IllegalStateException("Missing destination")
                true
            } catch (_: Exception) { false }
            runOnUiThread {
                if (pending !== result) return@runOnUiThread
                pending = null; content = null
                if (saved) result.success(true)
                else result.error("write", "Could not finish saving the document. Retry to a writable location.", null)
            }
        }
    }

    override fun onDestroy() {
        messagePrinter?.dispose(); messagePrinter = null
        pending?.error("closed", "The save was interrupted. Reopen Shep and retry.", null)
        pending = null; content = null
        writer.shutdown()
        super.onDestroy()
    }
    companion object { private const val SAVE_ATTACHMENT = 4107 }
}
