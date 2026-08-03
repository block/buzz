package xyz.block.buzz.mobile

import java.net.URI
import java.net.URLDecoder
import java.nio.charset.StandardCharsets

internal object AndroidNotificationContract {
    const val NOTIFICATION_TAP_ACTION =
        "xyz.block.buzz.mobile.action.NOTIFICATION_TAP"
    const val NOTIFICATION_ROUTE_EXTRA = "notification_route"

    private const val MAX_ROUTE_LENGTH = 2_048
    private const val MAX_ROUTE_VALUE_LENGTH = 512
    private val allowedQueryKeys = setOf("channel", "id", "thread")

    fun isTrustedTapIntent(
        action: String?,
        componentPackage: String?,
        componentClass: String?,
        applicationPackage: String,
        mainActivityClass: String,
    ): Boolean {
        return action == NOTIFICATION_TAP_ACTION &&
            componentPackage == applicationPackage &&
            componentClass == mainActivityClass
    }

    fun validatedMessageRoute(route: String?): String? {
        if (route.isNullOrEmpty() || route.length > MAX_ROUTE_LENGTH) return null
        if (route.any { it.isISOControl() || it.isWhitespace() }) return null

        val uri = try {
            URI(route)
        } catch (_: IllegalArgumentException) {
            return null
        }
        if (uri.isOpaque || uri.scheme != "buzz" || uri.rawAuthority != "message") return null
        if (!uri.rawPath.isNullOrEmpty() || uri.rawFragment != null || uri.rawUserInfo != null) {
            return null
        }
        if (uri.port != -1) return null

        val values = parseQuery(uri.rawQuery) ?: return null
        if (values.keys.any { it !in allowedQueryKeys }) return null
        if (values["channel"].isNullOrEmpty() || values["id"].isNullOrEmpty()) return null
        if ("thread" in values && values["thread"].isNullOrEmpty()) return null
        return route
    }

    fun isValidPermissionCallback(
        permissions: Array<out String>,
        grantResultCount: Int,
        notificationPermission: String,
    ): Boolean {
        return permissions.size == grantResultCount &&
            (permissions.isEmpty() || permissions.singleOrNull() == notificationPermission)
    }

    private fun parseQuery(rawQuery: String?): Map<String, String>? {
        if (rawQuery.isNullOrEmpty()) return null
        val values = mutableMapOf<String, String>()
        for (entry in rawQuery.split('&')) {
            if (entry.isEmpty()) return null
            val separator = entry.indexOf('=')
            if (separator <= 0) return null
            val key = decode(entry.substring(0, separator)) ?: return null
            val value = decode(entry.substring(separator + 1)) ?: return null
            if (key.isEmpty() || value.length > MAX_ROUTE_VALUE_LENGTH) return null
            if (key.any { it.isISOControl() || it.isWhitespace() } ||
                value.any { it.isISOControl() || it.isWhitespace() }
            ) {
                return null
            }
            if (values.put(key, value) != null) return null
        }
        return values
    }

    private fun decode(value: String): String? {
        return try {
            URLDecoder.decode(value, StandardCharsets.UTF_8.name())
        } catch (_: IllegalArgumentException) {
            null
        }
    }
}
