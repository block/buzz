package xyz.block.buzz.mobile

import android.app.Activity
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.content.res.ColorStateList
import android.graphics.Rect
import android.graphics.drawable.RippleDrawable
import android.widget.LinearLayout
import android.widget.PopupWindow
import android.widget.ScrollView
import android.widget.TextView
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodChannel
import kotlin.math.roundToInt

internal class ConversationPopup(private val activity: Activity, messenger: BinaryMessenger) {
    private val channel = MethodChannel(messenger, "buzz/conversation_popup")
    private var update: ((List<String>) -> Unit)? = null
    private var requestId: Int? = null
    private var dismiss: (() -> Unit)? = null

    init {
        channel.setMethodCallHandler { call, result ->
            when (call.method) {
                "update" -> {
                    if (call.argument<Int>("id") == requestId) {
                        update?.invoke(call.argument<List<String>>("labels").orEmpty())
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
                    fun dp(value: Int) = (value * scale).roundToInt()
                    val surface = call.argument<Number>("surface")!!.toInt()
                    val foreground = call.argument<Number>("foreground")!!.toInt()
                    val rows = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }
                    val scroll = ScrollView(activity).apply {
                        addView(rows)
                        background = GradientDrawable().apply {
                            cornerRadius = dp(18).toFloat()
                            setColor(surface)
                        }
                        clipToOutline = true
                    }
                    val popup = PopupWindow(scroll, dp(216).coerceAtMost(root.width), -2, true).apply {
                        setBackgroundDrawable(scroll.background.constantState?.newDrawable())
                        elevation = dp(8).toFloat()
                        isOutsideTouchable = true
                        inputMethodMode = PopupWindow.INPUT_METHOD_NOT_NEEDED
                        enterTransition = null
                        exitTransition = null
                    }
                    var selection: String? = null
                    var retired = false
                    fun finish() {
                        if (retired) return
                        retired = true
                        update = null
                        dismiss = null
                        root.removeView(anchor)
                        result.success(selection)
                    }
                    dismiss = { popup.dismiss(); finish() }
                    popup.setOnDismissListener { finish() }
                    fun populate(items: List<String>) {
                        rows.removeAllViews()
                        items.forEach { label ->
                            rows.addView(TextView(activity).apply {
                                text = label
                                textSize = 16f
                                gravity = Gravity.CENTER_VERTICAL
                                minHeight = dp(50)
                                setTextColor(foreground)
                                setPaddingRelative(dp(16), dp(12), dp(16), dp(12))
                                val icon = when (label) {
                                    "Mark read", "Mark unread" -> R.drawable.menu_mail
                                    "Rename" -> R.drawable.menu_rename
                                    "Archive", "Unarchive" -> R.drawable.menu_archive
                                    "Move to section…" -> R.drawable.menu_folder_input
                                    else -> R.drawable.menu_folder
                                }
                                setCompoundDrawablesRelativeWithIntrinsicBounds(icon, 0, 0, 0)
                                compoundDrawableTintList = ColorStateList.valueOf(foreground)
                                compoundDrawablePadding = dp(16)
                                background = RippleDrawable(ColorStateList.valueOf(
                                    (foreground and 0x00ffffff) or 0x22000000), null, null)
                                isFocusable = true
                                setOnClickListener { selection = label; popup.dismiss() }
                            })
                        }
                    }
                    fun place() {
                        rows.measure(View.MeasureSpec.makeMeasureSpec(popup.width, View.MeasureSpec.EXACTLY),
                            View.MeasureSpec.UNSPECIFIED)
                        val visible = Rect()
                        anchor.getWindowVisibleDisplayFrame(visible)
                        val pos = IntArray(2)
                        anchor.getLocationOnScreen(pos)
                        val below = visible.bottom - pos[1] - anchor.height - inset
                        val above = pos[1] - visible.top - inset
                        val upwards = below < rows.measuredHeight && above > below
                        val height = rows.measuredHeight.coerceAtMost((if (upwards) above else below).coerceAtLeast(1))
                        val offset = if (upwards) -anchor.height - height - inset else inset
                        if (popup.isShowing) popup.update(anchor, 0, offset, popup.width, height)
                        else {
                            popup.height = height
                            popup.animationStyle = if (upwards) R.style.ConversationPopupAbove else R.style.ConversationPopupBelow
                            popup.showAsDropDown(anchor, 0, offset, Gravity.START)
                        }
                    }
                    populate(labels)
                    update = { items ->
                        populate(items)
                        if (popup.isShowing) place()
                    }
                    anchor.addOnLayoutChangeListener(object : View.OnLayoutChangeListener {
                        override fun onLayoutChange(v: View, l: Int, t: Int, r: Int, b: Int, ol: Int, ot: Int, oldRight: Int, ob: Int) {
                            anchor.removeOnLayoutChangeListener(this)
                            if (!retired) place()
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
