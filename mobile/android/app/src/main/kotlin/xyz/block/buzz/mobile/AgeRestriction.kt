package xyz.block.buzz.mobile

/**
 * A confirmed age restriction, as push wakes see it.
 *
 * Like the iOS restriction session, it lives only as long as the process that
 * confirmed it, and nothing is saved: a later launch starts unrestricted and
 * checks again. Wakes run in the same process as the app, so while it holds
 * no wake posts anything.
 */
internal object AgeRestriction {
    private val lock = Any()

    // Guarded by lock.
    private var restricted = false

    /** Whether a wake should be skipped without fetching anything. */
    val isRestricted: Boolean
        get() = synchronized(lock) { restricted }

    /**
     * Restricts, then removes what is already posted with [cancelAll]. Posting
     * goes through [unlessRestricted], so nothing a wake posts outlives this.
     */
    fun restrict(cancelAll: () -> Unit) = synchronized(lock) {
        restricted = true
        cancelAll()
    }

    fun release() = synchronized(lock) { restricted = false }

    /** Runs [post] unless restricted, and never during [restrict]. */
    fun unlessRestricted(post: () -> Unit) = synchronized(lock) {
        if (!restricted) post()
    }
}
