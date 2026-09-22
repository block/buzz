package xyz.block.buzz.mobile

import com.google.firebase.appcheck.FirebaseAppCheck
import com.google.firebase.appcheck.debug.DebugAppCheckProviderFactory

internal object BuzzAppCheckProvider {
    fun install(appCheck: FirebaseAppCheck) {
        appCheck.installAppCheckProviderFactory(DebugAppCheckProviderFactory.getInstance())
    }
}
