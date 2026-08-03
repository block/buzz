package xyz.block.buzz.mobile

import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class AndroidNotificationContractTest {
    @Test
    fun `accepts only canonical message notification routes`() {
        val routes = listOf(
            "buzz://message?channel=channel-1&id=event-1",
            "buzz://message?channel=channel-1&id=event-1&thread=root-1",
            "buzz://message?channel=channel%2D1&id=event%2D1",
        )

        for (route in routes) {
            assertEquals(route, AndroidNotificationContract.validatedMessageRoute(route))
        }
    }

    @Test
    fun `rejects malformed or non-message notification routes`() {
        val routes = listOf<String?>(
            null,
            "",
            "https://example.com/message?channel=c&id=e",
            "buzz://join?channel=c&id=e",
            "BUZZ://message?channel=c&id=e",
            "buzz://MESSAGE?channel=c&id=e",
            "buzz://message/path?channel=c&id=e",
            "buzz://message?channel=c",
            "buzz://message?id=e",
            "buzz://message?channel=&id=e",
            "buzz://message?channel=c&id=",
            "buzz://message?channel=c&id=e&thread=",
            "buzz://message?channel=c&id=e&unexpected=value",
            "buzz://message?channel=c&channel=d&id=e",
            "buzz://message?channel=c&id=e#fragment",
            "buzz://message?channel=c%20d&id=e",
            "buzz://message?channel=c&id=%0Ae",
            "buzz://user@message?channel=c&id=e",
            "buzz://message:443?channel=c&id=e",
        )

        for (route in routes) {
            assertNull(
                AndroidNotificationContract.validatedMessageRoute(route),
                "expected route to be rejected: $route",
            )
        }
    }

    @Test
    fun `trusts tap actions only when they directly target the unexported activity`() {
        assertTrue(
            AndroidNotificationContract.isTrustedTapIntent(
                action = AndroidNotificationContract.NOTIFICATION_TAP_ACTION,
                componentPackage = "xyz.block.buzz.mobile",
                componentClass = "xyz.block.buzz.mobile.MainActivity",
                applicationPackage = "xyz.block.buzz.mobile",
                mainActivityClass = "xyz.block.buzz.mobile.MainActivity",
            ),
        )
        assertFalse(
            AndroidNotificationContract.isTrustedTapIntent(
                action = AndroidNotificationContract.NOTIFICATION_TAP_ACTION,
                componentPackage = "xyz.block.buzz.mobile",
                componentClass = "xyz.block.buzz.mobile.MainActivityAlias",
                applicationPackage = "xyz.block.buzz.mobile",
                mainActivityClass = "xyz.block.buzz.mobile.MainActivity",
            ),
        )
        assertFalse(
            AndroidNotificationContract.isTrustedTapIntent(
                action = "android.intent.action.VIEW",
                componentPackage = "xyz.block.buzz.mobile",
                componentClass = "xyz.block.buzz.mobile.MainActivity",
                applicationPackage = "xyz.block.buzz.mobile",
                mainActivityClass = "xyz.block.buzz.mobile.MainActivity",
            ),
        )
    }

    @Test
    fun `accepts completed and cancelled notification permission callbacks`() {
        val permission = "android.permission.POST_NOTIFICATIONS"
        assertTrue(
            AndroidNotificationContract.isValidPermissionCallback(
                arrayOf(permission),
                grantResultCount = 1,
                notificationPermission = permission,
            ),
        )
        assertTrue(
            AndroidNotificationContract.isValidPermissionCallback(
                emptyArray(),
                grantResultCount = 0,
                notificationPermission = permission,
            ),
        )
        assertFalse(
            AndroidNotificationContract.isValidPermissionCallback(
                arrayOf(permission),
                grantResultCount = 0,
                notificationPermission = permission,
            ),
        )
        assertFalse(
            AndroidNotificationContract.isValidPermissionCallback(
                arrayOf("android.permission.CAMERA"),
                grantResultCount = 1,
                notificationPermission = permission,
            ),
        )
    }

    @Test
    fun `notification icon is a white-only 24dp vector`() {
        val projectDir = File(assertNotNull(System.getProperty("buzz.android.appProjectDir")))
        val icon = File(projectDir, "src/main/res/drawable/ic_stat_buzz.xml")
        assertTrue(icon.isFile, "missing notification icon: $icon")

        val document = parseXml(icon)
        val androidNamespace = "http://schemas.android.com/apk/res/android"
        val vector = document.documentElement
        assertEquals("vector", vector.tagName)
        assertEquals("24dp", vector.getAttributeNS(androidNamespace, "width"))
        assertEquals("24dp", vector.getAttributeNS(androidNamespace, "height"))
        assertEquals("24", vector.getAttributeNS(androidNamespace, "viewportWidth"))
        assertEquals("24", vector.getAttributeNS(androidNamespace, "viewportHeight"))
        assertEquals(0, document.getElementsByTagName("gradient").length)

        val paths = document.getElementsByTagName("path")
        assertTrue(paths.length > 0)
        for (index in 0 until paths.length) {
            val fill = paths.item(index).attributes
                .getNamedItemNS(androidNamespace, "fillColor")
                ?.nodeValue
            assertTrue(fill == "#FFFFFFFF" || fill == "#FFFFFF")
        }
    }

    @Test
    fun `only the launcher alias is exported`() {
        val projectDir = File(assertNotNull(System.getProperty("buzz.android.appProjectDir")))
        val manifest = parseXml(File(projectDir, "src/main/AndroidManifest.xml"))
        val androidNamespace = "http://schemas.android.com/apk/res/android"

        val activities = manifest.getElementsByTagName("activity")
        val mainActivity = (0 until activities.length)
            .map { activities.item(it) }
            .single {
                it.attributes.getNamedItemNS(androidNamespace, "name")?.nodeValue ==
                    ".MainActivity"
            }
        assertEquals(
            "false",
            mainActivity.attributes.getNamedItemNS(androidNamespace, "exported")?.nodeValue,
        )

        val aliases = manifest.getElementsByTagName("activity-alias")
        val launcherAlias = (0 until aliases.length)
            .map { aliases.item(it) }
            .single {
                it.attributes.getNamedItemNS(androidNamespace, "targetActivity")?.nodeValue ==
                    ".MainActivity"
            }
        assertEquals(
            "true",
            launcherAlias.attributes.getNamedItemNS(androidNamespace, "exported")?.nodeValue,
        )
        assertEquals(
            ".MainActivityAlias",
            launcherAlias.attributes.getNamedItemNS(androidNamespace, "name")?.nodeValue,
        )
    }

    private fun parseXml(file: File) = DocumentBuilderFactory.newInstance().run {
        isNamespaceAware = true
        setFeature("http://apache.org/xml/features/disallow-doctype-decl", true)
        setAttribute("http://javax.xml.XMLConstants/property/accessExternalDTD", "")
        setAttribute("http://javax.xml.XMLConstants/property/accessExternalSchema", "")
        newDocumentBuilder().parse(file)
    }
}
