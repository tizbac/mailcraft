// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@file:Suppress("MaxLineLength", "TooManyFunctions")

package work.mailcraft.android

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog

private val Gutter = 16.dp
private val Gap = 12.dp
private val Spinner = 28.dp
private val ChipGap = 8.dp

/** Add Account: Microsoft first. Its three faces are the form, the wait, and what to do when an administrator has to approve. */
@Composable
internal fun AddAccountScreen(texts: Texts, state: AddAccountState, actions: AccountActions, onBack: () -> Unit) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    val editing = (state as? AddAccountState.Imap)?.editing != null || (state as? AddAccountState.Exchange)?.editing != null ||
                        (state as? AddAccountState.Dav)?.editing != null
                    Text(texts.t(if (editing) "Edit account" else "Add account"), maxLines = 1, overflow = TextOverflow.Ellipsis)
                },
                navigationIcon = {
                    IconButton(onClick = onBack) { Icon(AppIcons.Back, contentDescription = texts.t("Back")) }
                },
            )
        },
    ) { padding ->
        Column(
            modifier = Modifier.padding(padding).verticalScroll(rememberScrollState()).padding(Gutter),
            verticalArrangement = Arrangement.spacedBy(Gap),
        ) {
            when (state) {
                is AddAccountState.Form -> AddAccountForm(texts, state, actions)
                is AddAccountState.Imap -> ImapAccountForm(texts, state, actions)
                is AddAccountState.Exchange -> ExchangeAccountForm(texts, state, actions)
                is AddAccountState.Linchange -> LinchangeAccountForm(texts, state, actions)
                is AddAccountState.Dav -> DavAccountForm(texts, state, actions)
                is AddAccountState.Waiting -> SignInWaiting(texts, actions.onCancelSignIn)
                is AddAccountState.Approval -> ApprovalHelp(texts, state, actions)
                is AddAccountState.Done -> Unit
            }
        }
    }
}

@Composable
private fun AddAccountForm(texts: Texts, state: AddAccountState.Form, actions: AccountActions) {
    Text(texts.t("Microsoft account"), style = MaterialTheme.typography.titleMedium)
    Text(
        texts.t("Outlook.com, Hotmail, and Microsoft 365 work or school accounts. You sign in on Microsoft's own page in your browser, so MailCraft never sees your password."),
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    OutlinedTextField(
        value = state.email,
        onValueChange = actions.onEmail,
        modifier = Modifier.fillMaxWidth(),
        label = { Text(texts.t("Email address (optional)")) },
        supportingText = { Text(texts.t("Only used to fill in Microsoft's page for you.")) },
        singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email),
    )
    KeepOnPhone(texts, state.keep, actions)
    state.notice?.let { NoticeCard(it.text(texts)) }
    Button(onClick = actions.onSignIn, modifier = Modifier.fillMaxWidth()) {
        Text(texts.t("Sign in with Microsoft"))
    }
    OutlinedButton(onClick = actions.onChooseImap, modifier = Modifier.fillMaxWidth()) {
        Text(texts.t("Add an IMAP account"))
    }
    OutlinedButton(onClick = actions.onChooseExchange, modifier = Modifier.fillMaxWidth()) {
        Text(texts.t("Add an Exchange account"))
    }
    OutlinedButton(onClick = actions.onChooseLinchange, modifier = Modifier.fillMaxWidth()) {
        Text(texts.t("Add a Linchange account"))
    }
    OutlinedButton(onClick = actions.onChooseDav, modifier = Modifier.fillMaxWidth()) {
        Text(texts.t("Add calendars and contacts only (CalDAV / CardDAV)"))
    }
}

/** The Linchange form: the address, the server, and the wait while its own page approves MailCraft. */
@Composable
private fun LinchangeAccountForm(texts: Texts, state: AddAccountState.Linchange, actions: AccountActions) {
    Text(texts.t("Linchange account"), style = MaterialTheme.typography.titleMedium)
    Text(
        texts.t(
            "You sign in on Linchange's own page in your browser. It gives MailCraft an application password for this phone, so your account password and its second factor never reach MailCraft.",
        ),
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    OutlinedTextField(
        value = state.email,
        onValueChange = { actions.onLinchangeField(LinchangeField.Email, it) },
        modifier = Modifier.fillMaxWidth(),
        label = { Text(texts.t("Email address")) },
        singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email),
        enabled = !state.working,
    )
    OutlinedTextField(
        value = state.server,
        onValueChange = { actions.onLinchangeField(LinchangeField.Server, it) },
        modifier = Modifier.fillMaxWidth(),
        label = { Text(texts.t("Linchange server")) },
        supportingText = { Text(texts.t("The server that hosts your mail and Linchange page.")) },
        singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
        enabled = !state.working,
    )
    OutlinedTextField(
        value = state.displayName,
        onValueChange = { actions.onLinchangeField(LinchangeField.DisplayName, it) },
        modifier = Modifier.fillMaxWidth(),
        label = { Text(texts.t("Name (optional)")) },
        singleLine = true,
        enabled = !state.working,
    )
    state.notice?.let { NoticeCard(it.text(texts)) }
    if (state.working) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Gap)) {
            CircularProgressIndicator(modifier = Modifier.size(Spinner))
            Text(
                texts.t("Approve MailCraft on Linchange's page in your browser. MailCraft is waiting, and carries on by itself once you have."),
                modifier = Modifier.weight(1f),
            )
        }
        OutlinedButton(onClick = actions.onCancelLinchange, modifier = Modifier.fillMaxWidth()) {
            Text(texts.t("Cancel"))
        }
    } else {
        Button(onClick = actions.onAddLinchange, modifier = Modifier.fillMaxWidth()) {
            Text(texts.t("Approve in your browser"))
        }
    }
}

/** The CalDAV and CardDAV form: calendars and address books alone, with the homes they live at. */
@Composable
@Suppress("LongMethod")
private fun DavAccountForm(texts: Texts, state: AddAccountState.Dav, actions: AccountActions) {
    Text(texts.t("Calendars and contacts only"), style = MaterialTheme.typography.titleMedium)
    Text(
        texts.t(
            "An account with calendars and address books and no mail, kept in step over CalDAV and CardDAV. Your password goes to those servers only, and is kept on this phone when sign-ins are remembered.",
        ),
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    OutlinedTextField(
        value = state.email,
        onValueChange = { actions.onDavField(DavField.Email, it) },
        modifier = Modifier.fillMaxWidth(),
        label = { Text(texts.t("Email address or account name")) },
        singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email),
        enabled = !state.working,
    )
    OutlinedTextField(
        value = state.displayName,
        onValueChange = { actions.onDavField(DavField.DisplayName, it) },
        modifier = Modifier.fillMaxWidth(),
        label = { Text(texts.t("Name (optional)")) },
        singleLine = true,
        enabled = !state.working,
    )
    OutlinedTextField(
        value = state.username,
        onValueChange = { actions.onDavField(DavField.Username, it) },
        modifier = Modifier.fillMaxWidth(),
        label = { Text(texts.t("User name (optional)")) },
        supportingText = { Text(texts.t("The name the servers know you by; empty: the address above.")) },
        singleLine = true,
        enabled = !state.working,
    )
    OutlinedTextField(
        value = state.password,
        onValueChange = { actions.onDavField(DavField.Password, it) },
        modifier = Modifier.fillMaxWidth(),
        label = { Text(texts.t("Password")) },
        supportingText = if (state.editing != null) {
            { Text(texts.t("Leave empty to keep the password you have.")) }
        } else {
            null
        },
        singleLine = true,
        visualTransformation = PasswordVisualTransformation(),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
        enabled = !state.working,
    )
    OutlinedTextField(
        value = state.calendarHome,
        onValueChange = { actions.onDavField(DavField.CalendarHome, it) },
        modifier = Modifier.fillMaxWidth(),
        label = { Text(texts.t("Calendar address (CalDAV)")) },
        supportingText = { Text(texts.t("For example https://example.org/caldav/ — empty when it keeps no calendars.")) },
        singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
        enabled = !state.working,
    )
    OutlinedTextField(
        value = state.contactHome,
        onValueChange = { actions.onDavField(DavField.ContactHome, it) },
        modifier = Modifier.fillMaxWidth(),
        label = { Text(texts.t("Contacts address (CardDAV)")) },
        supportingText = { Text(texts.t("For example https://example.org/carddav/ — empty when it keeps no contacts.")) },
        singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
        enabled = !state.working,
    )
    state.notice?.let { NoticeCard(it.text(texts)) }
    Button(onClick = actions.onAddDav, modifier = Modifier.fillMaxWidth(), enabled = !state.working) {
        if (state.working) {
            CircularProgressIndicator(modifier = Modifier.size(Spinner))
        } else {
            Text(texts.t(if (state.editing == null) "Add account" else "Save"))
        }
    }
}

/** What the phone keeps of the new account's mail: the defaults chosen, so the owner can just go on. */
@Composable
private fun KeepOnPhone(texts: Texts, keep: KeepChoice, actions: AccountActions) {
    Text(texts.t("Mail on this phone"), style = MaterialTheme.typography.titleSmall)
    Text(
        texts.t("Recent mail is kept on this phone, and older mail stays on your mail server. Nothing is deleted from the server."),
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Text(texts.t("Keep mail from the last"), style = MaterialTheme.typography.bodyLarge)
    FlowRow(horizontalArrangement = Arrangement.spacedBy(ChipGap)) {
        KeepSpan.entries.forEach { span ->
            FilterChip(selected = keep.span == span, onClick = { actions.onKeepSpan(span) }, label = { Text(span.label(texts)) })
        }
    }
    Text(texts.t("Space for mail, at most"), style = MaterialTheme.typography.bodyLarge)
    FlowRow(horizontalArrangement = Arrangement.spacedBy(ChipGap)) {
        KeepChoice.STORAGE_STEPS.forEach { mb ->
            FilterChip(selected = keep.storageMb == mb, onClick = { actions.onStorage(mb) }, label = { Text(KeepChoice.storageLabel(texts, mb)) })
        }
    }
}

@Composable
internal fun NoticeCard(text: String) {
    Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.errorContainer)) {
        Text(
            text,
            modifier = Modifier.padding(Gutter),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onErrorContainer,
        )
    }
}

@Composable
private fun SignInWaiting(texts: Texts, onCancel: () -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Gap)) {
        CircularProgressIndicator(modifier = Modifier.size(Spinner))
        Text(texts.t("Finish signing in on Microsoft's page in your browser. MailCraft is waiting."), modifier = Modifier.weight(1f))
    }
    OutlinedButton(onClick = onCancel, modifier = Modifier.fillMaxWidth()) { Text(texts.t("Cancel")) }
}

/** "Need admin approval": what is going on, what to do next, and the message to hand to an administrator. */
@Composable
private fun ApprovalHelp(texts: Texts, state: AddAccountState.Approval, actions: AccountActions) {
    Text(texts.t("Your organisation needs to approve MailCraft"), style = MaterialTheme.typography.titleMedium)
    Text(
        texts.t("Many organisations let only an administrator approve a new program. Nothing is wrong with your account or your phone."),
        style = MaterialTheme.typography.bodyMedium,
    )
    Text(texts.t("What to do next"), style = MaterialTheme.typography.titleSmall)
    Text(texts.t("1. Copy the message below and send it to your organisation's administrator."), style = MaterialTheme.typography.bodyMedium)
    Text(texts.t("2. When they have approved MailCraft, come back here and sign in again."), style = MaterialTheme.typography.bodyMedium)
    val message = state.link?.let { adminMessage(texts, it) }
    if (message != null) {
        Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainer)) {
            Text(message, modifier = Modifier.padding(Gutter), style = MaterialTheme.typography.bodySmall)
        }
        Button(onClick = { actions.onCopyMessage(message) }, modifier = Modifier.fillMaxWidth()) {
            Text(texts.t("Copy message for an administrator"))
        }
        if (state.copied) {
            Text(
                texts.t("Copied. Paste it into mail or chat for your administrator."),
                color = MaterialTheme.colorScheme.primary,
                style = MaterialTheme.typography.bodyMedium,
            )
        }
    } else {
        NoticeCard(texts.t("The message for your administrator could not be made."))
    }
    OutlinedButton(onClick = actions.onSignIn, modifier = Modifier.fillMaxWidth()) { Text(texts.t("Sign in again")) }
}

/** What the Mail tab shows when there is no account yet. */
@Composable
internal fun NoAccount(texts: Texts, onAdd: () -> Unit, modifier: Modifier = Modifier) {
    Column(
        modifier = modifier.fillMaxWidth().padding(Gutter),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(Gap, Alignment.CenterVertically),
    ) {
        Text(texts.t("No account yet"), style = MaterialTheme.typography.titleMedium)
        Text(
            texts.t("Add a Microsoft account to read your mail here."),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Button(onClick = onAdd) { Text(texts.t("Add account")) }
    }
}

/** Settings › Accounts: each account with where its permissions stand, and Add account. */
@Composable
internal fun AccountsPage(texts: Texts, accounts: List<AccountItem>, ui: AccountsUi, actions: AccountActions, onAdd: () -> Unit) {
    LaunchedEffect(accounts) { actions.onShown(accounts) }
    accounts.forEach { account ->
        Column(modifier = Modifier.fillMaxWidth().padding(horizontal = Gutter, vertical = Gap)) {
            Text(account.name, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                account.email,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (account.kind == "microsoft") {
                PermissionKind.entries.forEach { kind ->
                    PermissionRow(texts, account, kind, ui.statuses[StatusKey(account.id, kind)] ?: PermissionStatus.Checking, actions) {
                        // Add Account opens with the address as its hint.
                        actions.onSignInAgain(account)
                        onAdd()
                    }
                }
            } else {
                Text(
                    texts.t(if (account.kind == "dav") "Calendars and contacts only" else "Signs in with a password"),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Row(horizontalArrangement = Arrangement.spacedBy(ChipGap)) {
                if (account.kind == "imap" || account.kind == "exchange" || account.kind == "dav") {
                    TextButton(onClick = {
                        actions.onEditAccount(account)
                        onAdd()
                    }) { Text(texts.t("Edit account")) }
                }
                TextButton(onClick = { actions.onRemoveAsk(account) }) { Text(texts.t("Remove account")) }
            }
        }
        HorizontalDivider()
    }
    TextButton(onClick = onAdd, modifier = Modifier.padding(horizontal = Gutter / 2)) { Text(texts.t("Add account")) }
}

@Composable
private fun PermissionRow(
    texts: Texts,
    account: AccountItem,
    kind: PermissionKind,
    status: PermissionStatus,
    actions: AccountActions,
    onSignInAgain: () -> Unit,
) {
    Row(modifier = Modifier.fillMaxWidth().padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(modifier = Modifier.weight(1f)) {
            Text(kind.label(texts), style = MaterialTheme.typography.bodyLarge)
            Text(statusText(texts, status), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        when (status) {
            PermissionStatus.NotAllowed -> TextButton(onClick = { actions.onAsk(account, kind) }) { Text(texts.t("Allow")) }
            PermissionStatus.SignInNeeded -> TextButton(onClick = onSignInAgain) { Text(texts.t("Sign in again")) }
            else -> Unit
        }
    }
}

/** [content] under what is wrong with keeping the mailbox, or with a sign-in the system cut short. */
@Composable
internal fun WithAppNotices(
    texts: Texts,
    ui: AccountsUi,
    actions: AccountActions,
    onRestart: () -> Unit,
    content: @Composable () -> Unit,
) {
    val notices = ui.savingOff || ui.saveFailed || ui.interrupted != null
    Column(modifier = Modifier.fillMaxSize()) {
        if (notices) {
            Column(modifier = Modifier.statusBarsPadding().padding(horizontal = Gutter, vertical = Gap)) {
                if (ui.savingOff) {
                    NoticeCard(texts.t("MailCraft couldn't set aside the saved mailbox it couldn't read, so changes aren't saved this time. Free up space and restart the app."))
                }
                if (ui.saveFailed) {
                    NoticeCard(texts.t("MailCraft couldn't save your latest changes yet. It will try again; if the app closes first they may be lost."))
                    TextButton(onClick = actions.onRetrySave) { Text(texts.t("Retry")) }
                }
                if (ui.interrupted != null) {
                    NoticeCard(texts.t("Android closed MailCraft while Microsoft's page was open, so that sign-in could not be finished. Nothing was added."))
                    Row {
                        TextButton(onClick = { actions.onRestartInterrupted(); onRestart() }) { Text(texts.t("Sign in again")) }
                        TextButton(onClick = actions.onDismissInterrupted) { Text(texts.t("Not now")) }
                    }
                }
            }
        }
        // (The notices took the status bar's room: the screen below is not padded for it twice.)
        val below = if (notices) Modifier.consumeWindowInsets(WindowInsets.statusBars) else Modifier
        Box(modifier = Modifier.weight(1f).then(below)) { content() }
    }
}
