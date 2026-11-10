package xyz.block.buzz.mobile

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class TransparentAvatarViewTest {
    @Test
    fun `only bundled HTTPS WebM URLs may enter the local document`() {
        val url = "https://dwwgwmfqqjotj.cloudfront.net/avatars/20260925T154625166Z/webm/gloopies/gloopies-13.webm"
        assertTrue(isTransparentAvatarVideo(url))
        listOf(
            url.replace("https:", "http:"),
            url.replace(".net/", ".net.evil.example/"),
            url.replace("/avatars/", "/other/"),
            url.replace("gloopies-13", "../gloopies-13"),
            "$url?redirect=https://evil.example",
            "$url#fragment",
            url.replace("gloopies-13", "\"><script>"),
            "file:///avatar.webm",
        ).forEach { assertFalse(it, isTransparentAvatarVideo(it)) }
    }
}
