// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
@file:Suppress("MaxLineLength")

package work.mailcraft.android

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp

private val FormGap = 12.dp
private val PortWidth = 96.dp
private val SmallSpinner = 18.dp

/** The form for a mail server that signs in with a password: IMAP to read, SMTP to send. */
@Composable
internal fun ImapAccountForm(texts: Texts, state: AddAccountState.Imap, actions: AccountActions) {
    Text(texts.t("IMAP account"), style = MaterialTheme.typography.titleMedium)
    Text(
        texts.t("An IMAP server to read mail, and an SMTP server to send it. Your password goes to those servers only, and is kept on this phone when sign-ins are remembered."),
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    AddressFields(texts, state, actions)
    Text(texts.t("Incoming mail (IMAP)"), style = MaterialTheme.typography.titleSmall)
    ServerRow(
        texts,
        host = state.incomingHost,
        onHost = { actions.onImapField(ImapField.IncomingHost, it) },
        port = state.incomingPort,
        onPort = { actions.onImapField(ImapField.IncomingPort, it) },
        enabled = !state.working,
    )
    TlsRow(
        checked = state.incomingTls,
        onChecked = actions.onImapIncomingTls,
        label = texts.t("Encrypted from the first byte (TLS)"),
        enabled = !state.working,
    )
    Text(texts.t("Outgoing mail (SMTP)"), style = MaterialTheme.typography.titleSmall)
    ServerRow(
        texts,
        host = state.outgoingHost,
        onHost = { actions.onImapField(ImapField.OutgoingHost, it) },
        port = state.outgoingPort,
        onPort = { actions.onImapField(ImapField.OutgoingPort, it) },
        enabled = !state.working,
    )
    TlsRow(
        checked = state.outgoingTls,
        onChecked = actions.onImapOutgoingTls,
        label = texts.t("Encrypted from the first byte (off uses STARTTLS)"),
        enabled = !state.working,
    )
    state.notice?.let { NoticeCard(it.text(texts)) }
    Button(onClick = actions.onAddImap, modifier = Modifier.fillMaxWidth(), enabled = !state.working) {
        if (state.working) {
            CircularProgressIndicator(modifier = Modifier.size(SmallSpinner))
        } else {
            Text(texts.t("Add account"))
        }
    }
}

/** The address and the password, and the two names the owner may choose to give. */
@Composable
private fun AddressFields(texts: Texts, state: AddAccountState.Imap, actions: AccountActions) {
    Column(verticalArrangement = Arrangement.spacedBy(FormGap)) {
        OutlinedTextField(
            value = state.email,
            onValueChange = { actions.onImapField(ImapField.Email, it) },
            modifier = Modifier.fillMaxWidth(),
            label = { Text(texts.t("Email address")) },
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email),
            enabled = !state.working,
        )
        OutlinedTextField(
            value = state.password,
            onValueChange = { actions.onImapField(ImapField.Password, it) },
            modifier = Modifier.fillMaxWidth(),
            label = { Text(texts.t("Password")) },
            singleLine = true,
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            enabled = !state.working,
        )
        OutlinedTextField(
            value = state.displayName,
            onValueChange = { actions.onImapField(ImapField.DisplayName, it) },
            modifier = Modifier.fillMaxWidth(),
            label = { Text(texts.t("Name (optional)")) },
            singleLine = true,
            enabled = !state.working,
        )
        OutlinedTextField(
            value = state.username,
            onValueChange = { actions.onImapField(ImapField.Username, it) },
            modifier = Modifier.fillMaxWidth(),
            label = { Text(texts.t("User name (optional, the address if empty)")) },
            singleLine = true,
            enabled = !state.working,
        )
    }
}

/** A server's address and its port. */
@Composable
private fun ServerRow(
    texts: Texts,
    host: String,
    onHost: (String) -> Unit,
    port: String,
    onPort: (String) -> Unit,
    enabled: Boolean,
) {
    Row(horizontalArrangement = Arrangement.spacedBy(FormGap), verticalAlignment = Alignment.CenterVertically) {
        OutlinedTextField(
            value = host,
            onValueChange = onHost,
            modifier = Modifier.weight(1f),
            label = { Text(texts.t("Server")) },
            singleLine = true,
            enabled = enabled,
        )
        OutlinedTextField(
            value = port,
            onValueChange = onPort,
            modifier = Modifier.width(PortWidth),
            label = { Text(texts.t("Port")) },
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
            enabled = enabled,
        )
    }
}

/** Whether a server is encrypted from the first byte. */
@Composable
private fun TlsRow(checked: Boolean, onChecked: (Boolean) -> Unit, label: String, enabled: Boolean) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(FormGap)) {
        Switch(checked = checked, onCheckedChange = onChecked, enabled = enabled)
        Text(label, style = MaterialTheme.typography.bodyMedium)
    }
}
