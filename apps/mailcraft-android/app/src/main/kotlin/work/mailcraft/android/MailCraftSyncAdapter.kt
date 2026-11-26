// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
package work.mailcraft.android

import android.accounts.Account
import android.app.Service
import android.content.AbstractThreadedSyncAdapter
import android.content.ContentProviderClient
import android.content.Context
import android.content.Intent
import android.content.SyncResult
import android.os.Bundle
import android.os.IBinder
import kotlinx.coroutines.runBlocking

/**
 * What the phone's Calendar and Contacts apps ask for when they sync a MailCraft account: the data
 * MailCraft already has, written to the system's providers. It is one way — nothing is read back
 * into MailCraft — and nothing reaches a server here: MailCraft's own Send/Receive does that, and
 * the phone merely shows the result.
 */
internal class MailCraftSyncAdapter(context: Context) : AbstractThreadedSyncAdapter(context, true, false) {
    override fun onPerformSync(
        account: Account,
        extras: Bundle?,
        authority: String?,
        provider: ContentProviderClient?,
        syncResult: SyncResult?,
    ) {
        val app = context.applicationContext as? MailCraftApplication ?: return
        if (account.type != MAILCRAFT_ACCOUNT_TYPE) return
        if (!SystemSyncPermissions.granted(context)) return
        runCatching {
            runBlocking {
                val core = app.coreForSystemSync() ?: return@runBlocking
                when (authority) {
                    android.provider.CalendarContract.AUTHORITY ->
                        SystemSync.exportCalendars(context, core, account.name)
                    android.provider.ContactsContract.AUTHORITY ->
                        SystemSync.exportContacts(context, core, account.name)
                }
            }
        }.onFailure {
            // (An answer the phone can't take is not a reason to stop: the next sync tries again.)
            syncResult?.stats?.numIoExceptions = (syncResult?.stats?.numIoExceptions ?: 0) + 1
        }
    }
}

/** One system sync service per kind of data: the Calendar app's and the Contacts app's. */
abstract class SyncService : Service() {
    private val adapter by lazy { MailCraftSyncAdapter(this) }

    override fun onBind(intent: Intent?): IBinder? = adapter.syncAdapterBinder
}

class CalendarSyncService : SyncService()

class ContactsSyncService : SyncService()
