// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
@file:Suppress("MaxLineLength", "MagicNumber")

package work.mailcraft.android

import android.Manifest
import android.content.ContentProviderOperation
import android.content.ContentUris
import android.content.ContentValues
import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.CalendarContract
import android.provider.ContactsContract
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.platform.LocalContext
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import uniffi.mailcraft_mobile.CalendarRecord
import uniffi.mailcraft_mobile.Core
import uniffi.mailcraft_mobile.CoreException
import uniffi.mailcraft_mobile.EventRecord
import uniffi.mailcraft_mobile.PersonRecord

/** What the phone's calendar and contacts are written with: the right to write them. */
internal object SystemSyncPermissions {
    val all = listOf(
        Manifest.permission.READ_CALENDAR,
        Manifest.permission.WRITE_CALENDAR,
        Manifest.permission.READ_CONTACTS,
        Manifest.permission.WRITE_CONTACTS,
    )

    fun missing(context: Context): List<String> =
        all.filter { ContextCompat.checkSelfPermission(context, it) != PackageManager.PERMISSION_GRANTED }

    fun granted(context: Context): Boolean = missing(context).isEmpty()
}

/** How far back and ahead the phone's calendar is filled, and how long one read may be. */
private const val DAYS_BACK = 365L
private const val DAYS_AHEAD = 730L
private const val DAYS_PER_READ = 90L

private val TIMED: DateTimeFormatter = DateTimeFormatter.ISO_LOCAL_DATE_TIME

/**
 * Writes what MailCraft keeps of its CalDAV and CardDAV accounts into the phone's own calendar and
 * contacts, under the MailCraft account the system knows: one way, read-only in the system's apps.
 * Nothing is fetched here and nothing is read back into MailCraft.
 */
@Suppress("TooManyFunctions")
internal object SystemSync {
    /**
     * Make the system accounts match the MailCraft accounts that keep calendars and address books,
     * then fill the phone's calendar and contacts when the owner has allowed that.
     */
    suspend fun reconcile(context: Context, core: Core) {
        // (The writing itself is the sync adapter's: the system's Calendar and Contacts apps ask
        // for it, and it has what a provider wants of a sync adapter. Here the accounts are made
        // to match, which needs no calendar or contacts right.)
        PhoneAccounts.reconcile(context, phoneAccounts(core))
    }

    /** The MailCraft accounts the system shows, one per account that keeps calendars or address books. */
    private suspend fun phoneAccounts(core: Core): List<PhoneAccount> =
        try {
            core.folders(0uL).accounts
                .filter { it.kind == "dav" }
                .map { account ->
                    val label = listOf(account.displayName, account.name, account.email).firstOrNull { it.isNotBlank() }.orEmpty()
                    PhoneAccount(account.email.ifBlank { label }, label)
                }
                .filter { it.email.isNotBlank() }
                .distinctBy { it.email }
        } catch (_: CoreException) {
            emptyList()
        }

    /** The phone's calendar rows and events for MailCraft's CalDAV calendars. */
    @Suppress("ReturnCount")
    suspend fun exportCalendars(context: Context, core: Core, only: String? = null) {
        if (!SystemSyncPermissions.granted(context)) return
        val dav = try {
            core.folders(0uL).accounts.filter { it.kind == "dav" && (only == null || it.email == only) }.associate { it.id to it.email }
        } catch (_: CoreException) {
            return
        }
        if (dav.isEmpty()) return
        val list = try {
            core.calendars()
        } catch (_: CoreException) {
            return
        }
        val kept = list.calendars.filter { dav.containsKey(it.account) }
        val resolver = context.contentResolver
        kept.forEach { calendar ->
            val account = dav[calendar.account] ?: return@forEach
            val id = ensureCalendar(context, account, calendar)
            if (id == null) return@forEach
            // (The phone's copy is MailCraft's: what it has now replaces what it had.)
            runCatching { resolver.delete(CalendarContract.Events.CONTENT_URI, "${CalendarContract.Events.CALENDAR_ID}=?", arrayOf(id.toString())) }
            val seen = HashSet<String>()
            readEvents(core, calendar.id).forEach { event ->
                val sync = eventSyncId(event)
                if (!seen.add(sync)) return@forEach
                insertEvent(context, id, account, event, sync)
            }
        }
    }

    /** One event as the system stores it, or nothing when its time can't be read. */
    private fun insertEvent(context: Context, calendarId: Long, account: String, event: EventRecord, sync: String) {
        val start = eventMillis(event.start, event.allDay) ?: return
        val end = eventMillis(event.end, event.allDay) ?: start
        val values = ContentValues().apply {
            put(CalendarContract.Events.CALENDAR_ID, calendarId)
            put(CalendarContract.Events.TITLE, event.title)
            put(CalendarContract.Events.EVENT_LOCATION, event.location)
            put(CalendarContract.Events.DTSTART, start)
            put(CalendarContract.Events.DTEND, if (end > start) end else start + 1)
            put(CalendarContract.Events.ALL_DAY, if (event.allDay) 1 else 0)
            put(CalendarContract.Events.EVENT_TIMEZONE, if (event.allDay) "UTC" else ZoneId.systemDefault().id)
            put(CalendarContract.Events.EVENT_END_TIMEZONE, if (event.allDay) "UTC" else ZoneId.systemDefault().id)
            put(CalendarContract.Events.STATUS, if (event.cancelled) CalendarContract.Events.STATUS_CANCELED else CalendarContract.Events.STATUS_CONFIRMED)
            put(
                CalendarContract.Events.AVAILABILITY,
                when (event.showAs) {
                    "free" -> CalendarContract.Events.AVAILABILITY_FREE
                    "tentative" -> CalendarContract.Events.AVAILABILITY_TENTATIVE
                    else -> CalendarContract.Events.AVAILABILITY_BUSY
                },
            )
            put(CalendarContract.Events.HAS_ALARM, 0)
            put(CalendarContract.Events.DIRTY, 0)
            put(CalendarContract.Events.DELETED, 0)
            put("_sync_id", sync)
        }
        runCatching { context.contentResolver.insert(eventsUri(account), values) }
    }

    /** The events of one of MailCraft's calendars in the window the phone shows. */
    private suspend fun readEvents(core: Core, calendar: ULong): List<EventRecord> {
        val today = LocalDate.now()
        val from = today.minusDays(DAYS_BACK)
        val to = today.plusDays(DAYS_AHEAD)
        val found = ArrayList<EventRecord>()
        var cursor = from
        while (cursor < to) {
            val end = minOfDate(cursor.plusDays(DAYS_PER_READ), to)
            try {
                core.calendar(cursor.toString(), end.toString()).events.filterTo(found) { it.calendar == calendar }
            } catch (_: CoreException) {
                return found
            }
            cursor = end.plusDays(1)
        }
        return found
    }

    /** The phone's row for a MailCraft calendar, made the first time and kept after. */
    private fun ensureCalendar(context: Context, account: String, calendar: CalendarRecord): Long? {
        val resolver = context.contentResolver
        val sync = "mailcraft:calendar:${calendar.id}"
        val existing = runCatching {
            resolver.query(calendarsUri(account), arrayOf(CalendarContract.Calendars._ID), "_sync_id=?", arrayOf(sync), null)?.use { cursor ->
                if (cursor.moveToFirst()) cursor.getLong(0) else null
            }
        }.getOrNull()
        val color = (calendar.color.toLong() and 0xFFFFFFL).toInt() or 0xFF000000.toInt()
        val values = ContentValues().apply {
            put(CalendarContract.Calendars.ACCOUNT_NAME, account)
            put(CalendarContract.Calendars.ACCOUNT_TYPE, MAILCRAFT_ACCOUNT_TYPE)
            put(CalendarContract.Calendars.NAME, calendar.name)
            put(CalendarContract.Calendars.CALENDAR_DISPLAY_NAME, calendar.name)
            put(CalendarContract.Calendars.CALENDAR_COLOR, color)
            // (MailCraft keeps it: the system's apps show it and don't change it.)
            put(CalendarContract.Calendars.CALENDAR_ACCESS_LEVEL, CalendarContract.Calendars.CAL_ACCESS_READ)
            put(CalendarContract.Calendars.OWNER_ACCOUNT, account)
            put(CalendarContract.Calendars.VISIBLE, if (calendar.visible) 1 else 0)
            put(CalendarContract.Calendars.SYNC_EVENTS, 1)
            put(CalendarContract.Calendars.CALENDAR_TIME_ZONE, ZoneId.systemDefault().id)
            put("_sync_id", sync)
        }
        return if (existing != null) {
            runCatching { resolver.update(ContentUris.withAppendedId(calendarsUri(account), existing), values, null, null) }
            existing
        } else {
            runCatching { resolver.insert(calendarsUri(account), values)?.let { ContentUris.parseId(it) } }.getOrNull()
        }
    }

    /** The phone's contacts for MailCraft's people, each in the account whose book keeps it. */
    @Suppress("LoopWithTooManyJumpStatements")
    suspend fun exportContacts(context: Context, core: Core, only: String? = null) {
        if (!SystemSyncPermissions.granted(context)) return
        val dav = try {
            core.folders(0uL).accounts.filter { it.kind == "dav" && (only == null || it.email == only) }.associate { it.id to it.email }
        } catch (_: CoreException) {
            return
        }
        if (dav.isEmpty()) return
        val byAccount = HashMap<ULong, MutableList<PersonRecord>>()
        var offset = 0u
        val page = 500u
        var total = Long.MAX_VALUE
        while (offset.toLong() < total) {
            val window = try {
                core.people("", offset, page)
            } catch (_: CoreException) {
                break
            }
            total = window.total.toLong()
            if (window.people.isEmpty()) break
            window.people.filter { dav.containsKey(it.account) }.forEach { byAccount.getOrPut(it.account) { ArrayList() }.add(it) }
            offset += window.people.size.toUInt()
        }
        dav.forEach { (id, name) ->
            eraseContacts(context, name)
            byAccount[id].orEmpty().chunked(60).forEach { people -> writePeople(context.contentResolver, name, people) }
        }
    }

    private fun writePeople(resolver: android.content.ContentResolver, account: String, people: List<PersonRecord>) {
        val ops = ArrayList<ContentProviderOperation>(people.size * 4)
        people.forEach { person ->
            val raw = ops.size
            ops.add(
                ContentProviderOperation.newInsert(rawContactsUri(account))
                    .withValue(ContactsContract.RawContacts.ACCOUNT_NAME, account)
                    .withValue(ContactsContract.RawContacts.ACCOUNT_TYPE, MAILCRAFT_ACCOUNT_TYPE)
                    .withValue(ContactsContract.RawContacts.AGGREGATION_MODE, ContactsContract.RawContacts.AGGREGATION_MODE_DISABLED)
                    .withValue(ContactsContract.RawContacts.RAW_CONTACT_IS_READ_ONLY, 1)
                    .build(),
            )
            if (person.name.isNotBlank()) {
                ops.add(
                    ContentProviderOperation.newInsert(ContactsContract.Data.CONTENT_URI)
                        .withValueBackReference(ContactsContract.Data.RAW_CONTACT_ID, raw)
                        .withValue(ContactsContract.Data.MIMETYPE, ContactsContract.CommonDataKinds.StructuredName.CONTENT_ITEM_TYPE)
                        .withValue(ContactsContract.CommonDataKinds.StructuredName.DISPLAY_NAME, person.name)
                        .build(),
                )
            }
            if (person.company.isNotBlank() || person.title.isNotBlank()) {
                ops.add(
                    ContentProviderOperation.newInsert(ContactsContract.Data.CONTENT_URI)
                        .withValueBackReference(ContactsContract.Data.RAW_CONTACT_ID, raw)
                        .withValue(ContactsContract.Data.MIMETYPE, ContactsContract.CommonDataKinds.Organization.CONTENT_ITEM_TYPE)
                        .withValue(ContactsContract.CommonDataKinds.Organization.COMPANY, person.company)
                        .withValue(ContactsContract.CommonDataKinds.Organization.TITLE, person.title)
                        .build(),
                )
            }
            person.emails.take(20).forEach { address ->
                ops.add(
                    ContentProviderOperation.newInsert(ContactsContract.Data.CONTENT_URI)
                        .withValueBackReference(ContactsContract.Data.RAW_CONTACT_ID, raw)
                        .withValue(ContactsContract.Data.MIMETYPE, ContactsContract.CommonDataKinds.Email.CONTENT_ITEM_TYPE)
                        .withValue(ContactsContract.CommonDataKinds.Email.ADDRESS, address)
                        .withValue(ContactsContract.CommonDataKinds.Email.TYPE, ContactsContract.CommonDataKinds.Email.TYPE_OTHER)
                        .build(),
                )
            }
            person.phones.take(20).forEach { number ->
                ops.add(
                    ContentProviderOperation.newInsert(ContactsContract.Data.CONTENT_URI)
                        .withValueBackReference(ContactsContract.Data.RAW_CONTACT_ID, raw)
                        .withValue(ContactsContract.Data.MIMETYPE, ContactsContract.CommonDataKinds.Phone.CONTENT_ITEM_TYPE)
                        .withValue(ContactsContract.CommonDataKinds.Phone.NUMBER, number)
                        .withValue(ContactsContract.CommonDataKinds.Phone.TYPE, ContactsContract.CommonDataKinds.Phone.TYPE_OTHER)
                        .build(),
                )
            }
        }
        runCatching { resolver.applyBatch(ContactsContract.AUTHORITY, ops) }
    }

    /** What is written for an account that went: its calendar, its events and its people. */
    fun erase(context: Context, account: String) {
        if (!SystemSyncPermissions.granted(context)) return
        val resolver = context.contentResolver
        runCatching {
            resolver.delete(calendarsUri(account), "${CalendarContract.Calendars.ACCOUNT_TYPE}=? AND ${CalendarContract.Calendars.ACCOUNT_NAME}=?", arrayOf(MAILCRAFT_ACCOUNT_TYPE, account))
        }
        eraseContacts(context, account)
    }

    private fun eraseContacts(context: Context, account: String) {
        runCatching {
            context.contentResolver.delete(
                rawContactsUri(account),
                "${ContactsContract.RawContacts.ACCOUNT_TYPE}=? AND ${ContactsContract.RawContacts.ACCOUNT_NAME}=?",
                arrayOf(MAILCRAFT_ACCOUNT_TYPE, account),
            )
        }
    }

    private fun calendarsUri(account: String): Uri = CalendarContract.Calendars.CONTENT_URI.buildUpon()
        .appendQueryParameter(CalendarContract.CALLER_IS_SYNCADAPTER, "true")
        .appendQueryParameter(CalendarContract.Calendars.ACCOUNT_NAME, account)
        .appendQueryParameter(CalendarContract.Calendars.ACCOUNT_TYPE, MAILCRAFT_ACCOUNT_TYPE)
        .build()

    private fun eventsUri(account: String): Uri = CalendarContract.Events.CONTENT_URI.buildUpon()
        .appendQueryParameter(CalendarContract.CALLER_IS_SYNCADAPTER, "true")
        .appendQueryParameter(CalendarContract.Events.ACCOUNT_NAME, account)
        .appendQueryParameter(CalendarContract.Events.ACCOUNT_TYPE, MAILCRAFT_ACCOUNT_TYPE)
        .build()

    // (A raw contact is made under this program's own account type with the rights the owner gave;
    // the sync adapter's marker is not needed for a provider the app owns the data in.)
    private fun rawContactsUri(account: String): Uri = ContactsContract.RawContacts.CONTENT_URI.buildUpon()
        .appendQueryParameter(ContactsContract.RawContacts.ACCOUNT_NAME, account)
        .appendQueryParameter(ContactsContract.RawContacts.ACCOUNT_TYPE, MAILCRAFT_ACCOUNT_TYPE)
        .build()

    /** A stable name for one occurrence, so that reading it again doesn't make a second one. */
    internal fun eventSyncId(event: EventRecord): String = "mailcraft:event:${event.event}:${event.start}"

    private fun eventMillis(text: String, allDay: Boolean): Long? = runCatching {
        if (allDay) {
            LocalDate.parse(text.take(10)).atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli()
        } else {
            LocalDateTime.parse(text.take(19), TIMED).atZone(ZoneId.systemDefault()).toInstant().toEpochMilli()
        }
    }.getOrNull()

    private fun minOfDate(a: LocalDate, b: LocalDate): LocalDate = if (a.isBefore(b)) a else b
}

/**
 * Asks the owner, once a DAV account exists, for the right to put its calendars and contacts in the
 * phone's own apps. A no is kept by the system; MailCraft simply writes nothing.
 */
@Composable
internal fun AskForSystemSyncOnce(hasDavAccount: Boolean, onAnswered: () -> Unit = {}) {
    val context = LocalContext.current
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { onAnswered() }
    LaunchedEffect(hasDavAccount) {
        if (!hasDavAccount) return@LaunchedEffect
        val missing = SystemSyncPermissions.missing(context)
        if (missing.isNotEmpty()) ask.launch(missing.toTypedArray())
    }
}
