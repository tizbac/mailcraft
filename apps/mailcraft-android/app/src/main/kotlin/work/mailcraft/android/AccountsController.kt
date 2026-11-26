// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
@file:Suppress("LargeClass", "MaxLineLength")

package work.mailcraft.android

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import uniffi.mailcraft_mobile.CoreException
import uniffi.mailcraft_mobile.DavAccountSettings
import uniffi.mailcraft_mobile.ExchangeAccountSettings
import uniffi.mailcraft_mobile.ImapAccountSettings
import uniffi.mailcraft_mobile.MailServer

/**
 * What happens on the account screens: Add Account, the permission questions, Remove account. It
 * holds the state they draw ([ui]) and talks to the core only through [AccountsApi], so it runs
 * without the library. The core keeps every token; nothing here sees one.
 *
 * [accounts] is the list as the app has it now, [changed] is called after the mailbox's accounts
 * changed (to keep the mailbox and read the list again), [copy] puts text on the clipboard, and
 * [texts] says the words of a notice. [signedIn] is told of every sign-in that worked, after the
 * screen has moved on: the account, what the phone keeps of its mail, and whether it is new (not in
 * [accounts] before), so that its first Send/Receive starts at once.
 */
@Suppress("TooManyFunctions", "LongParameterList")
internal class AccountsController(
    private val api: AccountsApi,
    private val scope: CoroutineScope,
    private val texts: Texts,
    private val accounts: () -> List<AccountItem>,
    private val changed: suspend () -> Unit,
    private val copy: (String) -> Unit,
    private val retrySave: suspend () -> Unit = {},
    private val signedIn: suspend (account: ULong, keep: KeepChoice, added: Boolean) -> Unit = { _, _, _ -> },
) {
    val ui = MutableStateFlow(AccountsUi())

    fun actions(): AccountActions = AccountActions(
        onEmail = ::typed,
        onKeepSpan = { span -> chose { it.copy(span = span) } },
        onStorage = { mb -> chose { it.copy(storageMb = mb) } },
        onSignIn = ::signIn,
        onCancelSignIn = ::cancelSignIn,
        onCopyMessage = ::copyMessage,
        onLeaveAdd = ::leaveAdd,
        onChooseImap = ::chooseImap,
        onImapField = ::imapField,
        onImapIncomingTls = ::imapIncomingTls,
        onImapOutgoingTls = ::imapOutgoingTls,
        onAddImap = ::saveImap,
        onChooseExchange = ::chooseExchange,
        onExchangeField = ::exchangeField,
        onAddExchange = ::saveExchange,
        onChooseLinchange = ::chooseLinchange,
        onLinchangeField = ::linchangeField,
        onAddLinchange = ::saveLinchange,
        onCancelLinchange = api::cancelLinchange,
        onChooseDav = ::chooseDav,
        onDavField = ::davField,
        onAddDav = ::saveDav,
        onEditAccount = ::editAccount,
        onSignInAgain = ::signInAgain,
        onAsk = { account, permission -> askFor(account.id, permission) },
        onAllow = ::allow,
        onNotNow = ::notNow,
        onCancelGrant = api::cancel,
        onRemoveAsk = ::removeAsk,
        onRemove = ::remove,
        onRemoveCancel = ::removeCancel,
        onShown = ::refreshStatuses,
        onRestartInterrupted = ::restartInterrupted,
        onDismissInterrupted = ::dismissInterrupted,
        onRetrySave = { scope.launch { retrySave() } },
    )

    /** The kept mailbox can't be written this session: the screens say so. */
    fun setSavingOff(off: Boolean) = ui.update { it.copy(savingOff = off) }

    /** The last save of the mailbox failed (or worked again): the screens say it isn't saved yet. */
    fun setSaveFailed(failed: Boolean) = ui.update { it.copy(saveFailed = failed) }

    /**
     * Android closed the app while the provider's page was open and the owner came back: the sign-in
     * that was under way is gone with the process, so the screens say so and offer a new one.
     */
    fun signInInterrupted(email: String) = ui.update { state ->
        if (state.add is AddAccountState.Waiting) state else state.copy(interrupted = email.take(MAX_EMAIL))
    }

    private fun dismissInterrupted() = ui.update { it.copy(interrupted = null) }

    /** The form, its address as the hint, for the sign-in that was interrupted. */
    private fun restartInterrupted() = ui.update { state ->
        val email = state.interrupted ?: return@update state
        if (state.add is AddAccountState.Waiting) {
            state.copy(interrupted = null)
        } else {
            state.copy(add = AddAccountState.Form(email), interrupted = null)
        }
    }

    private fun typed(email: String) = ui.update { state ->
        val add = state.add
        if (add is AddAccountState.Form) {
            state.copy(add = add.copy(email = email.take(MAX_EMAIL), notice = null))
        } else {
            state
        }
    }

    /** What the phone keeps of the new account's mail, changed on the form. */
    private fun chose(change: (KeepChoice) -> KeepChoice) = ui.update { state ->
        val add = state.add
        if (add is AddAccountState.Form) state.copy(add = add.copy(keep = change(add.keep))) else state
    }

    /** The form for a mail server that signs in with a password, its servers guessed from the address. */
    private fun chooseImap() = ui.update { state ->
        val email = (state.add as? AddAccountState.Form)?.email.orEmpty()
        val domain = email.substringAfter('@', "").trim()
        val prefix = if (domain.isEmpty()) "" else "."
        state.copy(
            add = AddAccountState.Imap(
                email = email,
                incomingHost = if (domain.isEmpty()) "" else "imap$prefix$domain",
                outgoingHost = if (domain.isEmpty()) "" else "smtp$prefix$domain",
            ),
        )
    }

    /** One field of the mail-server form, typed: held within what a server name or a password may be. */
    private fun imapField(field: ImapField, typed: String) = ui.update { state ->
        val add = state.add as? AddAccountState.Imap ?: return@update state
        val room = when (field) {
            ImapField.Password -> MAX_PASSWORD
            ImapField.IncomingPort, ImapField.OutgoingPort -> MAX_PORT_DIGITS
            ImapField.Email -> MAX_EMAIL
            else -> MAX_HOST
        }
        val value = typed.take(room)
        state.copy(
            add = when (field) {
                ImapField.Email -> add.copy(email = value, notice = null)
                ImapField.DisplayName -> add.copy(displayName = value, notice = null)
                ImapField.Username -> add.copy(username = value, notice = null)
                ImapField.Password -> add.copy(password = value, notice = null)
                ImapField.IncomingHost -> add.copy(incomingHost = value, notice = null)
                ImapField.IncomingPort -> add.copy(incomingPort = value, notice = null)
                ImapField.OutgoingHost -> add.copy(outgoingHost = value, notice = null)
                ImapField.OutgoingPort -> add.copy(outgoingPort = value, notice = null)
            },
        )
    }

    /** Whether the incoming server is encrypted from the first byte; its usual port follows. */
    private fun imapIncomingTls(on: Boolean) = ui.update { state ->
        val add = state.add as? AddAccountState.Imap ?: return@update state
        val port = if (add.incomingPort == (if (on) "143" else "993")) (if (on) "993" else "143") else add.incomingPort
        state.copy(add = add.copy(incomingTls = on, incomingPort = port, notice = null))
    }

    /** Whether the outgoing server is encrypted from the first byte; its usual port follows. */
    private fun imapOutgoingTls(on: Boolean) = ui.update { state ->
        val add = state.add as? AddAccountState.Imap ?: return@update state
        val port = if (add.outgoingPort == (if (on) "587" else "465")) (if (on) "465" else "587") else add.outgoingPort
        state.copy(add = add.copy(outgoingTls = on, outgoingPort = port, notice = null))
    }

    /** Adds the account the mail-server form describes; a password never leaves the core after this. */
    /** Add an IMAP account, or change one the form opened for: its servers are tried first. */
    private fun saveImap() {
        val add = ui.value.add as? AddAccountState.Imap ?: return
        if (add.working) return
        val ready = imapReady(add) ?: return
        ui.update { state ->
            (state.add as? AddAccountState.Imap)?.let {
                state.copy(add = it.copy(working = true, notice = null))
            } ?: state
        }
        scope.launch {
            val settings = imapSettings(ready.add, ready.incoming, ready.outgoing)
            val editing = add.editing
            // A new account's servers and password are tried before it is added, as the desktop
            // checks them; an edit is tried when a password is given (an empty one keeps the old).
            if (editing == null || add.password.isNotEmpty()) {
                try {
                    api.checkImap(settings)
                } catch (e: CoreException) {
                    failedImap(e.what())
                    return@launch
                }
            }
            val id = try {
                if (editing == null) {
                    api.addImap(settings)
                } else {
                    api.editImap(editing, settings)
                    editing
                }
            } catch (e: CoreException) {
                failedImap(e.what())
                return@launch
            }
            changed()
            ui.update { it.copy(add = AddAccountState.Done(id)) }
            // The first (or next) Send/Receive goes through the app's account path, as a sign-in's does.
            scope.launch { signedIn(id, KeepChoice(), editing == null) }
        }
    }

    /** The form for an account the owner has, filled with what the core says it has. */
    private fun editAccount(account: AccountItem) {
        scope.launch {
            val settings = try {
                api.settings(account.id)
            } catch (_: CoreException) {
                return@launch
            }
            ui.update { state ->
                state.copy(
                    add = when (account.kind) {
                        "imap" -> AddAccountState.Imap(
                            email = settings.email,
                            displayName = settings.displayName,
                            username = settings.username,
                            incomingHost = settings.incoming?.host.orEmpty(),
                            incomingPort = settings.incoming?.port?.toString() ?: "993",
                            incomingTls = settings.incoming?.security != "starttls",
                            outgoingHost = settings.outgoing?.host.orEmpty(),
                            outgoingPort = settings.outgoing?.port?.toString() ?: "587",
                            outgoingTls = settings.outgoing?.security != "starttls",
                            editing = account.id,
                        )
                        "exchange" -> AddAccountState.Exchange(
                            email = settings.email,
                            displayName = settings.displayName,
                            username = settings.username,
                            url = settings.url,
                            editing = account.id,
                        )
                        "dav" -> AddAccountState.Dav(
                            email = settings.email,
                            displayName = settings.displayName,
                            username = settings.username,
                            calendarHome = settings.calendarHomes.firstOrNull().orEmpty(),
                            contactHome = settings.contactHomes.firstOrNull().orEmpty(),
                            otherCalendarHomes = settings.calendarHomes.drop(1),
                            otherContactHomes = settings.contactHomes.drop(1),
                            editing = account.id,
                        )
                        else -> return@update state
                    },
                )
            }
        }
    }

    private data class ImapReady(val add: AddAccountState.Imap, val incoming: UShort, val outgoing: UShort)

    /** The form's fields as a whole, or null with what it still lacks said on it. */
    private fun imapReady(add: AddAccountState.Imap): ImapReady? {
        val incoming = add.incomingPort.toUShortOrNull()
        val outgoing = add.outgoingPort.toUShortOrNull()
        val problem = imapProblem(add, incoming, outgoing)
        if (problem != null) refuseImap(problem)
        return if (problem == null && incoming != null && outgoing != null) ImapReady(add, incoming, outgoing) else null
    }

    /** What the form still lacks, or null when it can be sent; the ports are checked apart. */
    private fun imapProblem(add: AddAccountState.Imap, incoming: UShort?, outgoing: UShort?): String? = when {
        add.email.trim().isEmpty() -> "Type the account's address."
        add.password.isEmpty() && add.editing == null -> "Type the account's password."
        add.incomingHost.isBlank() -> "Type the incoming server's address."
        add.outgoingHost.isBlank() -> "Type the outgoing server's address."
        incoming == null || incoming == 0u.toUShort() -> "Type the incoming server's port."
        outgoing == null || outgoing == 0u.toUShort() -> "Type the outgoing server's port."
        else -> null
    }

    private fun imapSettings(add: AddAccountState.Imap, incoming: UShort, outgoing: UShort) = ImapAccountSettings(
        email = add.email.trim(),
        displayName = add.displayName.trim(),
        username = add.username.trim(),
        password = add.password,
        incoming = MailServer(add.incomingHost.trim(), incoming, if (add.incomingTls) "tls" else "starttls"),
        outgoing = MailServer(add.outgoingHost.trim(), outgoing, if (add.outgoingTls) "tls" else "starttls"),
    )

    private fun refuseImap(problem: String) = ui.update { state ->
        (state.add as? AddAccountState.Imap)?.let {
            state.copy(add = it.copy(notice = Notice.Failed(problem)))
        } ?: state
    }

    private fun failedImap(reason: String) = ui.update { state ->
        (state.add as? AddAccountState.Imap)?.let {
            state.copy(add = it.copy(working = false, notice = Notice.Failed(reason)))
        } ?: state
    }

    /** The form for an Exchange server account, its server guessed from the address. */
    private fun chooseExchange() = ui.update { state ->
        val email = (state.add as? AddAccountState.Form)?.email.orEmpty()
        val domain = email.substringAfter('@', "").trim()
        state.copy(
            add = AddAccountState.Exchange(
                email = email,
                url = if (domain.isEmpty()) "" else "https://$domain/EWS/Exchange.asmx",
            ),
        )
    }

    private fun exchangeField(field: ExchangeField, typed: String) = ui.update { state ->
        val add = state.add as? AddAccountState.Exchange ?: return@update state
        val room = when (field) {
            ExchangeField.Password -> MAX_PASSWORD
            ExchangeField.Email -> MAX_EMAIL
            ExchangeField.Url -> MAX_URL
            else -> MAX_HOST
        }
        val value = typed.take(room)
        state.copy(
            add = when (field) {
                ExchangeField.Email -> add.copy(email = value, notice = null)
                ExchangeField.DisplayName -> add.copy(displayName = value, notice = null)
                ExchangeField.Username -> add.copy(username = value, notice = null)
                ExchangeField.Password -> add.copy(password = value, notice = null)
                ExchangeField.Url -> add.copy(url = value, notice = null)
            },
        )
    }

    /** Add an Exchange server account, or change one the form opened for. */
    private fun saveExchange() {
        val add = ui.value.add as? AddAccountState.Exchange ?: return
        if (add.working) return
        val problem = exchangeProblem(add)
        if (problem != null) {
            refuseExchange(problem)
            return
        }
        ui.update { state ->
            (state.add as? AddAccountState.Exchange)?.let {
                state.copy(add = it.copy(working = true, notice = null))
            } ?: state
        }
        scope.launch {
            val editing = add.editing
            val id = try {
                if (editing == null) {
                    api.addExchange(exchangeSettings(add))
                } else {
                    api.editExchange(editing, exchangeSettings(add))
                    editing
                }
            } catch (e: CoreException) {
                failedExchange(e.what())
                return@launch
            }
            changed()
            ui.update { it.copy(add = AddAccountState.Done(id)) }
            scope.launch { signedIn(id, KeepChoice(), editing == null) }
        }
    }

    /** What the Exchange form still lacks, or null when it can be sent. */
    private fun exchangeProblem(add: AddAccountState.Exchange): String? = when {
        add.email.trim().isEmpty() -> "Type the account's address."
        add.password.isEmpty() && add.editing == null -> "Type the account's password."
        add.url.trim().isEmpty() -> "Type the server's Exchange Web Services address."
        !add.url.trim().startsWith("https://") && !add.url.trim().startsWith("http://127.0.0.1:") ->
            "The server's address is an https:// address."
        else -> null
    }

    private fun exchangeSettings(add: AddAccountState.Exchange) = ExchangeAccountSettings(
        email = add.email.trim(),
        displayName = add.displayName.trim(),
        username = add.username.trim(),
        password = add.password,
        url = add.url.trim(),
    )

    private fun refuseExchange(problem: String) = ui.update { state ->
        (state.add as? AddAccountState.Exchange)?.let {
            state.copy(add = it.copy(notice = Notice.Failed(problem)))
        } ?: state
    }

    private fun failedExchange(reason: String) = ui.update { state ->
        (state.add as? AddAccountState.Exchange)?.let {
            state.copy(add = it.copy(working = false, notice = Notice.Failed(reason)))
        } ?: state
    }

    /** The form for a Linchange account, its server guessed from the address's domain. */
    private fun chooseLinchange() = ui.update { state ->
        val email = (state.add as? AddAccountState.Form)?.email.orEmpty()
        state.copy(add = AddAccountState.Linchange(email = email, server = linchangeServerFor(email)))
    }

    /** The Linchange server a domain usually has: `mail.<domain>`, unless that is the domain. */
    private fun linchangeServerFor(email: String): String {
        val domain = email.substringAfter('@', "").trim()
        return if (domain.isEmpty() || domain.startsWith("mail.")) domain else "mail.$domain"
    }

    private fun linchangeField(field: LinchangeField, typed: String) = ui.update { state ->
        val add = state.add as? AddAccountState.Linchange ?: return@update state
        val room = if (field == LinchangeField.Email) MAX_EMAIL else MAX_HOST
        val value = typed.take(room)
        state.copy(
            add = when (field) {
                LinchangeField.Email -> {
                    // (The server follows the address until the owner types one of their own.)
                    val follows = add.server.isBlank() || add.server == linchangeServerFor(add.email) || add.server == add.email.substringAfter('@', "").trim()
                    add.copy(email = value, server = if (follows) linchangeServerFor(value) else add.server, notice = null)
                }
                LinchangeField.DisplayName -> add.copy(displayName = value, notice = null)
                LinchangeField.Server -> add.copy(server = value, notice = null)
            },
        )
    }

    /** Asks Linchange's page for an application password, then adds the account with it. */
    @Suppress("CyclomaticComplexMethod")
    private fun saveLinchange() {
        val add = ui.value.add as? AddAccountState.Linchange ?: return
        if (add.working) return
        val email = add.email.trim()
        val server = add.server.trim()
        val problem = when {
            email.isEmpty() -> "Type the account's address."
            else -> null
        }
        if (problem != null) {
            ui.update { state ->
                (state.add as? AddAccountState.Linchange)?.let { state.copy(add = it.copy(notice = Notice.Failed(problem))) } ?: state
            }
            return
        }
        ui.update { state ->
            (state.add as? AddAccountState.Linchange)?.let { state.copy(add = it.copy(working = true, notice = null)) } ?: state
        }
        scope.launch {
            val result = api.addLinchange(
                email = email,
                displayName = add.displayName.trim(),
                server = server,
                description = texts.t("MailCraft on this phone"),
            )
            when (result) {
                is LinchangeResult.SignedIn -> {
                    changed()
                    ui.update { it.copy(add = AddAccountState.Done(result.account)) }
                    scope.launch { signedIn(result.account, KeepChoice(), true) }
                }
                LinchangeResult.Cancelled -> failedLinchange(texts.t("The Linchange approval was called off."))
                LinchangeResult.TimedOut -> failedLinchange(texts.t("The Linchange approval wasn't finished in time. You can try again."))
                LinchangeResult.NoBrowser -> failedLinchange(texts.t("MailCraft couldn't open a browser for Linchange's page."))
                is LinchangeResult.Failed -> failedLinchange(result.reason.ifBlank { texts.t("The Linchange approval didn't finish. You can try again.") })
            }
        }
    }

    private fun failedLinchange(reason: String) = ui.update { state ->
        (state.add as? AddAccountState.Linchange)?.let {
            state.copy(add = it.copy(working = false, notice = Notice.Failed(reason)))
        } ?: state
    }

    /** The form for calendars and address books alone: nothing is guessed, since nothing detects it. */
    private fun chooseDav() = ui.update { state ->
        val email = (state.add as? AddAccountState.Form)?.email.orEmpty()
        state.copy(add = AddAccountState.Dav(email = email))
    }

    private fun davField(field: DavField, typed: String) = ui.update { state ->
        val add = state.add as? AddAccountState.Dav ?: return@update state
        val room = when (field) {
            DavField.Email -> MAX_EMAIL
            DavField.Password -> MAX_PASSWORD
            DavField.CalendarHome, DavField.ContactHome -> MAX_URL
            else -> MAX_HOST
        }
        val value = typed.take(room)
        state.copy(
            add = when (field) {
                DavField.Email -> add.copy(email = value, notice = null)
                DavField.DisplayName -> add.copy(displayName = value, notice = null)
                DavField.Username -> add.copy(username = value, notice = null)
                DavField.Password -> add.copy(password = value, notice = null)
                DavField.CalendarHome -> add.copy(calendarHome = value, notice = null)
                DavField.ContactHome -> add.copy(contactHome = value, notice = null)
            },
        )
    }

    /** What the CalDAV and CardDAV form still lacks, or null when it can be sent. */
    private fun davProblem(add: AddAccountState.Dav): String? = when {
        add.email.trim().isEmpty() -> "Type the account's name or address."
        add.calendarHome.trim().isEmpty() && add.contactHome.trim().isEmpty() -> "Type a calendar or contacts address."
        add.password.isEmpty() && add.editing == null -> "Type the account's password."
        !add.calendarHome.trim().isEmpty() && !add.calendarHome.trim().startsWith("https://") -> "The calendar address is an https:// address."
        !add.contactHome.trim().isEmpty() && !add.contactHome.trim().startsWith("https://") -> "The contacts address is an https:// address."
        else -> null
    }

    private fun davSettings(add: AddAccountState.Dav) = DavAccountSettings(
        email = add.email.trim(),
        displayName = add.displayName.trim(),
        username = add.username.trim(),
        password = add.password,
        calendarHomes = listOfNotNull(add.calendarHome.trim().takeIf { it.isNotEmpty() }) + add.otherCalendarHomes,
        contactHomes = listOfNotNull(add.contactHome.trim().takeIf { it.isNotEmpty() }) + add.otherContactHomes,
    )

    /** Add a CalDAV and CardDAV account, or change one the form opened for. */
    private fun saveDav() {
        val add = ui.value.add as? AddAccountState.Dav ?: return
        if (add.working) return
        val problem = davProblem(add)
        if (problem != null) {
            failedDav(problem)
            return
        }
        ui.update { state ->
            (state.add as? AddAccountState.Dav)?.let { state.copy(add = it.copy(working = true, notice = null)) } ?: state
        }
        scope.launch {
            val editing = add.editing
            val id = try {
                if (editing == null) api.addDav(davSettings(add)) else editing.also { api.editDav(it, davSettings(add)) }
            } catch (e: CoreException) {
                failedDav(e.what())
                return@launch
            }
            changed()
            ui.update { it.copy(add = AddAccountState.Done(id)) }
            scope.launch { signedIn(id, KeepChoice(), editing == null) }
        }
    }

    private fun failedDav(reason: String) = ui.update { state ->
        (state.add as? AddAccountState.Dav)?.let {
            state.copy(add = it.copy(working = false, notice = Notice.Failed(reason)))
        } ?: state
    }

    private fun keepOf(add: AddAccountState): KeepChoice = when (add) {
        is AddAccountState.Form -> add.keep
        is AddAccountState.Imap -> KeepChoice()
        is AddAccountState.Exchange -> KeepChoice()
        is AddAccountState.Linchange -> KeepChoice()
        is AddAccountState.Dav -> KeepChoice()
        is AddAccountState.Approval -> add.keep
        is AddAccountState.Waiting -> add.keep
        is AddAccountState.Done -> KeepChoice()
    }

    private fun emailOf(add: AddAccountState): String? = when (add) {
        is AddAccountState.Form -> add.email
        is AddAccountState.Imap -> add.email
        is AddAccountState.Exchange -> add.email
        is AddAccountState.Linchange -> add.email
        is AddAccountState.Dav -> add.email
        is AddAccountState.Approval -> add.email
        is AddAccountState.Waiting, is AddAccountState.Done -> null
    }

    /** Which sign-in owns the Add Account screen: a result of an earlier one is never shown. */
    private var signInOperation = 0L

    /** The last sign-in started: the next waits for it to be over. */
    private var signInJob: Job? = null

    /** The sign-in that is waiting for an earlier one to be over, and hasn't opened its page; 0 when none is. */
    private var queued = 0L

    /** Cancel on the waiting screen: the sign-in under way is called off, and one still queued never begins. */
    private fun cancelSignIn() {
        api.cancel()
        if (queued != 0L && queued == signInOperation) {
            queued = 0L
            signInOperation++
            ui.update { state ->
                val add = state.add
                if (add is AddAccountState.Waiting) {
                    state.copy(add = AddAccountState.Form(add.email, Notice.Cancelled, add.keep))
                } else {
                    state
                }
            }
        }
    }

    /** Opens Microsoft's page and waits for the owner. Only from the form or the approval help. */
    @Suppress("CyclomaticComplexMethod")
    fun signIn() {
        if (ui.value.add is AddAccountState.Exchange || ui.value.add is AddAccountState.Linchange || ui.value.add is AddAccountState.Dav) return
        val email = emailOf(ui.value.add) ?: return
        val keep = keepOf(ui.value.add)
        val operation = ++signInOperation
        ui.update { it.copy(add = AddAccountState.Waiting(email, keep)) }
        val previous = signInJob
        signInJob = scope.launch {
            // A sign-in the owner left (leaving the screen calls it off) is over before this one begins:
            // one at a time, so that its end can't call off, or disarm, the new one.
            if (previous?.isActive == true) {
                queued = operation
                api.cancel()
                previous.join()
                if (queued == operation) queued = 0L
            }
            // (Called off, or left for a newer one, while it waited: it never opens its page.)
            if (operation != signInOperation) return@launch
            // (The accounts before the sign-in: the list may be read again before it returns.)
            val before = accounts().map { it.id }.toSet()
            val result = api.signIn(email, keep)
            val link = if (result is SignInResult.NeedsApproval) api.approvalLink(email) else null
            // (Whether the account was there before: a new one has its first sync go through the room gate.)
            val added = result is SignInResult.SignedIn && result.account !in before
            if (result is SignInResult.SignedIn) {
                // The sign-in is new: where this account's permissions stand is asked again, now (the
                // Accounts page may stay on screen with the same list, which asks nothing by itself).
                PermissionKind.entries.forEach { setStatus(result.account, it) }
                changed()
            }
            ui.update { state ->
                // (The owner may have left the screen meanwhile: what is there is left as it is.)
                val current = operation == signInOperation && state.add is AddAccountState.Waiting
                if (current) state.copy(add = afterSignIn(email, result, link, keep)) else state
            }
            // Mail starts to come at once (a new account, or one signed in again), not at the next
            // periodic run: the screen has closed by now, and the list shows the round under way.
            // (Apart from this sign-in's job, so that a room question left open never holds up the next.)
            if (result is SignInResult.SignedIn) scope.launch { signedIn(result.account, keep, added) }
        }
    }

    private fun copyMessage(text: String) {
        copy(text)
        ui.update { state ->
            val add = state.add
            if (add is AddAccountState.Approval) state.copy(add = add.copy(copied = true)) else state
        }
    }

    /** Leaves Add Account: the form is as new, and a sign-in still queued there never begins. */
    private fun leaveAdd() {
        queued = 0L
        signInOperation++
        if (ui.value.add is AddAccountState.Linchange) api.cancelLinchange()
        ui.update { it.copy(add = AddAccountState.Form()) }
    }

    /** The form for an account the owner has, its address as the hint; a sign-in that waits is left as it is. */
    private fun signInAgain(account: AccountItem) = ui.update { state ->
        if (state.add is AddAccountState.Waiting) state else state.copy(add = AddAccountState.Form(account.email))
    }

    /** Puts the question for a permission to the owner (an event from the core, or the owner's own tap). */
    fun askFor(account: ULong, permission: PermissionKind) = ui.update { state ->
        if (state.question != null || accounts().none { it.id == account }) {
            state
        } else {
            state.copy(question = QuestionState.Asking(PermissionAsk(account, permission)))
        }
    }

    /** The core says an account's sign-in is over. */
    fun signInNeeded(account: ULong) = ui.update { state ->
        val over = PermissionKind.entries.associate { StatusKey(account, it) to PermissionStatus.SignInNeeded }
        // (An answer still on its way is older than this and must not replace it.)
        over.keys.forEach { generations[it] = (generations[it] ?: 0) + 1 }
        state.copy(statuses = state.statuses + over)
    }

    /** The owner said yes: Microsoft's page opens. */
    private fun allow() {
        val asking = ui.value.question as? QuestionState.Asking ?: return
        val ask = asking.ask
        ui.update { it.copy(question = QuestionState.Waiting(ask)) }
        scope.launch {
            val result = api.grant(ask.account, ask.permission)
            val told = grantNotice(texts, result)
            ui.update { state -> state.copy(question = told?.let { QuestionState.Told(ask, it) }) }
            if (result is GrantResult.Granted) setStatus(ask.account, ask.permission)
        }
    }

    private fun notNow() = ui.update { state ->
        if (state.question is QuestionState.Waiting) state else state.copy(question = null)
    }

    private fun removeAsk(account: AccountItem) = ui.update { it.copy(removing = account, removeFailed = false) }

    private fun removeCancel() = ui.update { it.copy(removing = null, removeFailed = false) }

    /** The accounts whose removal is under way: a second tap on Remove for one of them does nothing. */
    private val removals = HashSet<ULong>()

    private fun remove() {
        val account = ui.value.removing ?: return
        if (!removals.add(account.id)) return
        scope.launch {
            var removed = false
            try {
                removed = api.remove(account.id)
            } finally {
                removals.remove(account.id)
            }
            // The confirmation is this removal's only while it is still the one for this account: one
            // the owner dismissed, or another account's opened meanwhile, is left as it is.
            fun owns(state: AccountsUi) = state.removing?.id == account.id
            // (Only a removal that happened cleans up; a failed one says so and can be tried again.)
            if (!removed) {
                ui.update { state -> if (owns(state)) state.copy(removeFailed = true) else state }
                return@launch
            }
            ui.update { state ->
                val closed = if (owns(state)) state.copy(removing = null, removeFailed = false) else state
                closed.copy(statuses = closed.statuses.filterKeys { it.account != account.id })
            }
            changed()
        }
    }

    /** Asks the core where each permission of each account stands (the Accounts page is on screen). */
    private fun refreshStatuses(shown: List<AccountItem>) {
        shown.filter { it.kind == "microsoft" }.forEach { account ->
            PermissionKind.entries.forEach { kind ->
                val key = StatusKey(account.id, kind)
                // (Not known yet, or couldn't be told the last time.)
                val known = ui.value.statuses[key]
                if (known == null || known == PermissionStatus.Unknown) setStatus(account.id, kind)
            }
        }
    }

    private fun setStatus(account: ULong, permission: PermissionKind) {
        val key = StatusKey(account, permission)
        val generation = (generations[key] ?: 0) + 1
        generations[key] = generation
        ui.update { it.copy(statuses = it.statuses + (key to PermissionStatus.Checking)) }
        scope.launch {
            val status = api.status(account, permission)
            // (An account removed meanwhile has no row to keep, and a newer word about it stands.)
            ui.update { state ->
                val current = key in state.statuses && generations[key] == generation
                if (current) state.copy(statuses = state.statuses + (key to status)) else state
            }
        }
    }

    /** How many times each status was asked or overridden: a late answer to an older ask is dropped. */
    private val generations = HashMap<StatusKey, Int>()

    private companion object {
        const val MAX_EMAIL = 254

        /** The longest a server's name, a user name or a display name may be. */
        const val MAX_HOST = 255

        /** The most a password may be (the platform's store takes a bounded secret). */
        const val MAX_PASSWORD = 1_024

        /** The most digits a port may be typed in (65535 is five). */
        const val MAX_PORT_DIGITS = 5

        /** The longest a server's Exchange Web Services address may be. */
        const val MAX_URL = 2_048
    }
}

/** What a core error says, in the owner's words (uniffi's `message` puts the field's name first). */
internal fun CoreException.what(): String = (this as? CoreException.Failed)?.reason.orEmpty()
