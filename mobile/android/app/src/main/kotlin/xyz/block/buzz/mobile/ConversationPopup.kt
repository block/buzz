package xyz.block.buzz.mobile

import android.app.Activity
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.PopupMenu
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodChannel
import kotlin.math.roundToInt

internal class ConversationPopup(private val activity: Activity, messenger: BinaryMessenger) {
    private val channel = MethodChannel(messenger, "buzz/conversation_popup")
    private var popupMenu: PopupMenu? = null
    private var requestId: Int? = null
    private var dismiss: (() -> Unit)? = null

    init {
        channel.setMethodCallHandler { call, result ->
            when (call.method) {
                "update" -> {
                    if (call.argument<Int>("id") == requestId) popupMenu?.menu?.let { menu ->
                        menu.clear()
                        call.argument<List<String>>("labels")?.forEach { menu.add(it) }
                    }
                    result.success(null)
                }
                "dismiss" -> {
                    if (requestId == call.argument<Int>("id")) dismiss?.invoke()
                    result.success(null)
                }
                "show" -> {
                    dismiss?.invoke()
                    requestId = call.argument<Int>("id")
                    val labels = call.argument<List<String>>("labels").orEmpty()
                    val root = activity.findViewById<FrameLayout>(android.R.id.content)
                    if (activity.isFinishing || !root.isAttachedToWindow || labels.isEmpty()) {
                        result.success(null)
                        return@setMethodCallHandler
                    }
                    val scale = root.resources.displayMetrics.density
                    fun px(key: String) = ((call.argument<Number>(key)?.toFloat() ?: 0f) * scale).roundToInt()
                    val inset = (12 * scale).roundToInt()
                    val anchor = View(activity).apply {
                        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
                        background = GradientDrawable().apply {
                            cornerRadius = 12 * scale
                            setColor(0x227F7F7F)
                        }
                    }
                    val frame = FrameLayout.LayoutParams((px("width") - 2 * inset).coerceAtLeast(1),
                        px("height").coerceAtLeast(1), Gravity.TOP or Gravity.LEFT)
                    frame.leftMargin = px("x") + inset
                    frame.topMargin = px("y")
                    root.addView(anchor, frame)
                    val popup = PopupMenu(activity, anchor, Gravity.START)
                    popupMenu = popup
                    var selection: String? = null
                    var retired = false
                    fun finish() {
                        if (retired) return
                        retired = true
                        popupMenu = null
                        dismiss = null
                        root.removeView(anchor)
                        result.success(selection)
                    }
                    dismiss = { popup.dismiss(); finish() }
                    labels.forEach { popup.menu.add(it) }
                    popup.setOnMenuItemClickListener {
                        selection = it.title.toString()
                        true
                    }
                    popup.setOnDismissListener { finish() }
                    anchor.addOnLayoutChangeListener(object : View.OnLayoutChangeListener {
                        override fun onLayoutChange(v: View, l: Int, t: Int, r: Int, b: Int, ol: Int, ot: Int, oldRight: Int, ob: Int) {
                            anchor.removeOnLayoutChangeListener(this)
                            if (!retired) popup.show()
                        }
                    })
                }
                else -> result.notImplemented()
            }
        }
    }

    fun dispose() {
        dismiss?.invoke()
        channel.setMethodCallHandler(null)
    }
}
