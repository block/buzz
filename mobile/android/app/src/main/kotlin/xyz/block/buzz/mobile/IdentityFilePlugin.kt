package xyz.block.buzz.mobile

import android.app.Activity
import android.content.Intent
import android.net.Uri
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import java.util.concurrent.Executors

/** Transfers identity files directly through the document picker, never cache. */
internal class IdentityFilePlugin(
    private val activity: Activity,
    messenger: BinaryMessenger,
) : MethodChannel.MethodCallHandler {
    private val channel = MethodChannel(messenger, "buzz/identity_file")
        .apply { setMethodCallHandler(this@IdentityFilePlugin) }
    private val worker = Executors.newSingleThreadExecutor()
    private class Request(val result: MethodChannel.Result, val importing: Boolean, var bytes: ByteArray?) {
        val failure get() = if (importing) "import_failed" else "export_failed"
    }
    private var pending: Request? = null

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        if (call.method != "export" && call.method != "import") {
            result.notImplemented()
            return
        }
        if (pending != null) {
            result.error(if (call.method == "import") "import_busy" else "export_busy", null, null)
            return
        }
        val importing = call.method == "import"
        val args = call.arguments as? Map<*, *>
        val text = args?.get("text") as? String
        val name = args?.get("name") as? String
        if (!importing && (text == null || text.length !in 1..1024 || name == null ||
            !name.matches(Regex("[A-Za-z0-9._-]{1,80}")))) {
            result.error("invalid_input", null, null)
            return
        }
        pending = Request(result, importing, if (importing) null else text!!.toByteArray(Charsets.UTF_8))
        try {
            activity.startActivityForResult(Intent(
                if (importing) Intent.ACTION_OPEN_DOCUMENT else Intent.ACTION_CREATE_DOCUMENT,
            ).apply {
                addCategory(Intent.CATEGORY_OPENABLE)
                // Exported .nsec files may be classified as application/octet-stream.
                type = if (importing) "*/*" else "text/plain"
                if (!importing) putExtra(Intent.EXTRA_TITLE, name)
            }, REQUEST_CODE)
        } catch (_: Exception) {
            finishFailure()
        }
    }

    fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?): Boolean {
        if (requestCode != REQUEST_CODE) return false
        val request = pending ?: return true
        if (resultCode != Activity.RESULT_OK) {
            clear()
            request.result.success(if (request.importing) null else false)
            return true
        }
        val uri: Uri = data?.data ?: run {
            finishFailure()
            return true
        }
        // Keep pending set until completion, preventing overlapping transfers.
        val content = request.bytes
        request.bytes = null
        worker.execute {
            var value: Any? = null
            val successful = try {
                if (request.importing) {
                    val input = activity.contentResolver.openInputStream(uri)
                        ?: throw IllegalStateException("Document unavailable")
                    value = IdentityFileReader.read(input)
                } else {
                    requireNotNull(content)
                    activity.contentResolver.openOutputStream(uri, "wt")?.use { it.write(content) }
                        ?: throw IllegalStateException("Document unavailable")
                    value = true
                }
                true
            } catch (_: Exception) {
                false
            } finally {
                content?.fill(0)
            }
            activity.runOnUiThread {
                if (pending === request) {
                    clear()
                    if (successful) request.result.success(value)
                    else request.result.error(request.failure, null, null)
                }
            }
        }
        return true
    }

    private fun clear() {
        pending?.bytes?.fill(0)
        pending = null
    }

    private fun finishFailure() {
        val request = pending
        clear()
        request?.result?.error(request.failure, null, null)
    }

    fun dispose() {
        channel.setMethodCallHandler(null)
        finishFailure()
        worker.shutdown()
    }

    companion object {
        private const val REQUEST_CODE = 4627
    }
}
