// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
package work.mailcraft.android

import android.annotation.SuppressLint
import android.content.Context

/** Which accounts' folders are collapsed in the drawer: on this device only (nothing is backed up). */
internal interface FolderPrefs {
    fun load(): Set<ULong>

    fun save(collapsed: Set<ULong>)
}

/** [FolderPrefs] in the app's private preferences. A store that cannot be read gives no collapsed account. */
internal class DeviceFolderPrefs(context: Context) : FolderPrefs {
    private val prefs = runCatching { context.getSharedPreferences(FILE, Context.MODE_PRIVATE) }.getOrNull()

    override fun load(): Set<ULong> = runCatching {
        prefs?.getStringSet(COLLAPSED, null).orEmpty().mapNotNull { it.toULongOrNull() }.toSet()
    }.getOrDefault(emptySet())

    // (The KTX `edit {}` would be a dependency for one line.)
    @SuppressLint("UseKtx")
    override fun save(collapsed: Set<ULong>) {
        // (SharedPreferences keeps its own copy of the set, and refuses a null one.)
        runCatching { prefs?.edit()?.putStringSet(COLLAPSED, collapsed.mapTo(HashSet()) { it.toString() })?.apply() }
    }

    private companion object {
        const val FILE = "mail_list"
        const val COLLAPSED = "collapsed_accounts"
    }
}

/** Keeps the collapsed accounts in memory: for tests and screenshots. */
internal class MemoryFolderPrefs(private var collapsed: Set<ULong> = emptySet()) : FolderPrefs {
    override fun load(): Set<ULong> = collapsed

    override fun save(collapsed: Set<ULong>) {
        this.collapsed = collapsed.toSet()
    }
}
