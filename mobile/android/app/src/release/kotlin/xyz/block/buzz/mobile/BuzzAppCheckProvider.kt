package xyz.block.buzz.mobile

import com.google.firebase.appcheck.FirebaseAppCheck
import com.google.firebase.appcheck.playintegrity.PlayIntegrityAppCheckProviderFactory

internal object BuzzAppCheckProvider {
    fun install(appCheck: FirebaseAppCheck) {
        appCheck.installAppCheckProviderFactory(PlayIntegrityAppCheckProviderFactory.getInstance())
    }
}
