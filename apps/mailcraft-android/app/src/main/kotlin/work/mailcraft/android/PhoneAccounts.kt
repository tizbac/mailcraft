// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
package work.mailcraft.android

import android.accounts.Account
import android.accounts.AccountManager
import android.content.ContentResolver
import android.content.Context
import android.provider.CalendarContract
import android.provider.ContactsContract

/** The Android account type MailCraft's calendars and contacts appear under. */
internal const val MAILCRAFT_ACCOUNT_TYPE = "work.mailcraft.android"

/** How often the system may ask MailCraft to refresh the phone's calendar and contacts (seconds). */
private const val PERIOD_SECONDS = 6L * 60L * 60L

/** One MailCraft account as the phone sees it: the name the system account is kept under. */
internal data class PhoneAccount(val email: String, val label: String)

/**
 * The phone's own accounts for the MailCraft accounts that keep calendars and address books:
 * created when such an account is added, removed when it goes. The system's Calendar and Contacts
 * apps see them and call [MailCraftSyncAdapter] for what to show; nothing here is a sign-in.
 */
internal object PhoneAccounts {
    fun manager(context: Context): AccountManager? = runCatching { AccountManager.get(context) }.getOrNull()

    fun androidAccount(name: String): Account = Account(name, MAILCRAFT_ACCOUNT_TYPE)

    /** The system accounts of MailCraft's type, by name. */
    fun existing(context: Context): List<Account> =
        runCatching { manager(context)?.getAccountsByType(MAILCRAFT_ACCOUNT_TYPE)?.toList() }.getOrNull().orEmpty()

    /**
     * Makes the system's accounts match [wanted]: one for each, and none for an account that went.
     * Whether the phone's calendar and contacts may be written is the owner's to say; only the
     * account itself is made here.
     */
    fun reconcile(context: Context, wanted: List<PhoneAccount>) {
        val manager = manager(context) ?: return
        val byName = existing(context).associateBy { it.name }
        wanted.forEach { wantedAccount ->
            val account = byName[wantedAccount.email] ?: androidAccount(wantedAccount.email)
            if (byName[wantedAccount.email] == null) {
                runCatching { manager.addAccountExplicitly(account, null, null) }
            }
            // (The system's apps may ask for a refresh, and the calendar and contacts are kept in
            // step by this program's own runs: the periodic one is a safety net.)
            runCatching {
                ContentResolver.setIsSyncable(account, CalendarContract.AUTHORITY, 1)
                ContentResolver.setSyncAutomatically(account, CalendarContract.AUTHORITY, true)
                ContentResolver.addPeriodicSync(
                    account,
                    CalendarContract.AUTHORITY,
                    android.os.Bundle(),
                    PERIOD_SECONDS,
                )
                ContentResolver.setIsSyncable(account, ContactsContract.AUTHORITY, 1)
                ContentResolver.setSyncAutomatically(account, ContactsContract.AUTHORITY, true)
                ContentResolver.addPeriodicSync(
                    account,
                    ContactsContract.AUTHORITY,
                    android.os.Bundle(),
                    PERIOD_SECONDS,
                )
            }
        }
        val doomed = existing(context).filter { account -> wanted.none { it.email == account.name } }
        doomed.forEach { account ->
            // (What was written for it goes with the system account.)
            runCatching { SystemSync.erase(context, account.name) }
            runCatching { manager.removeAccountExplicitly(account) }
        }
    }

    /** Ask the system to run a sync for [name] now, for the two kinds of data MailCraft shows. */
    fun requestSync(context: Context, name: String? = null) {
        val accounts = if (name == null) existing(context) else listOf(androidAccount(name))
        accounts.forEach { account ->
            runCatching { ContentResolver.requestSync(account, CalendarContract.AUTHORITY, android.os.Bundle()) }
            runCatching { ContentResolver.requestSync(account, ContactsContract.AUTHORITY, android.os.Bundle()) }
        }
    }
}
