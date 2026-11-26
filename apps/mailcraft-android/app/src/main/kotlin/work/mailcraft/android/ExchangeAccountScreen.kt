// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
@file:Suppress("MaxLineLength")

package work.mailcraft.android

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp

private val ExchangeGap = 12.dp
private val ExchangeSpinner = 18.dp

/** The form for an Exchange server account: Exchange Web Services, with the password. */
@Composable
internal fun ExchangeAccountForm(texts: Texts, state: AddAccountState.Exchange, actions: AccountActions) {
    Text(texts.t("Exchange server account"), style = MaterialTheme.typography.titleMedium)
    Text(
        texts.t("An organisation's own Exchange Server, Grommunio or SmarterMail, through Exchange Web Services. Your password goes to that server only, and is kept on this phone when sign-ins are remembered."),
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    ExchangeFields(texts, state, actions)
    state.notice?.let { NoticeCard(it.text(texts)) }
    Button(onClick = actions.onAddExchange, modifier = Modifier.fillMaxWidth(), enabled = !state.working) {
        if (state.working) {
            CircularProgressIndicator(modifier = Modifier.size(ExchangeSpinner))
        } else {
            Text(texts.t(if (state.editing == null) "Add account" else "Save"))
        }
    }
}

/** The address, the password, the names and the server's Exchange Web Services address. */
@Composable
private fun ExchangeFields(texts: Texts, state: AddAccountState.Exchange, actions: AccountActions) {
    Column(verticalArrangement = Arrangement.spacedBy(ExchangeGap)) {
        OutlinedTextField(
            value = state.email,
            onValueChange = { actions.onExchangeField(ExchangeField.Email, it) },
            modifier = Modifier.fillMaxWidth(),
            label = { Text(texts.t("Email address")) },
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email),
            enabled = !state.working,
        )
        OutlinedTextField(
            value = state.password,
            onValueChange = { actions.onExchangeField(ExchangeField.Password, it) },
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
            value = state.username,
            onValueChange = { actions.onExchangeField(ExchangeField.Username, it) },
            modifier = Modifier.fillMaxWidth(),
            label = { Text(texts.t("User name (optional, the address if empty)")) },
            singleLine = true,
            enabled = !state.working,
        )
        OutlinedTextField(
            value = state.displayName,
            onValueChange = { actions.onExchangeField(ExchangeField.DisplayName, it) },
            modifier = Modifier.fillMaxWidth(),
            label = { Text(texts.t("Name (optional)")) },
            singleLine = true,
            enabled = !state.working,
        )
        OutlinedTextField(
            value = state.url,
            onValueChange = { actions.onExchangeField(ExchangeField.Url, it) },
            modifier = Modifier.fillMaxWidth(),
            label = { Text(texts.t("Exchange Web Services address")) },
            supportingText = { Text(texts.t("For example https://mail.example.org/EWS/Exchange.asmx")) },
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
            enabled = !state.working,
        )
    }
}
