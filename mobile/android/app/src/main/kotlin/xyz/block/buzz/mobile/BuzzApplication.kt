package xyz.block.buzz.mobile

import android.app.Application
import com.google.firebase.FirebaseApp
import com.google.firebase.FirebaseOptions
import com.google.firebase.appcheck.FirebaseAppCheck

class BuzzApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        BuzzFirebaseConfiguration.initialize(this)
    }
}

internal object BuzzFirebaseConfiguration {
    @Volatile
    private var initialized = false

    @Synchronized
    fun initialize(application: android.app.Application): Boolean {
        if (initialized) return true
        val values = listOf(
            BuildConfig.BUZZ_FIREBASE_PROJECT_ID,
            BuildConfig.BUZZ_FIREBASE_APPLICATION_ID,
            BuildConfig.BUZZ_FIREBASE_API_KEY,
            BuildConfig.BUZZ_FIREBASE_SENDER_ID,
        )
        if (values.any(String::isBlank)) return false
        val app = FirebaseApp.getApps(application).firstOrNull()
            ?: FirebaseApp.initializeApp(
                application,
                FirebaseOptions.Builder()
                    .setProjectId(BuildConfig.BUZZ_FIREBASE_PROJECT_ID)
                    .setApplicationId(BuildConfig.BUZZ_FIREBASE_APPLICATION_ID)
                    .setApiKey(BuildConfig.BUZZ_FIREBASE_API_KEY)
                    .setGcmSenderId(BuildConfig.BUZZ_FIREBASE_SENDER_ID)
                    .build(),
            )
            ?: return false
        BuzzAppCheckProvider.install(FirebaseAppCheck.getInstance(app))
        initialized = true
        return true
    }
}
