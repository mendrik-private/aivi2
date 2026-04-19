# aivi.gnome.onlineAccounts

Types for GNOME Online Accounts (GOA).

GNOME Online Accounts is the system service that keeps track of signed-in desktop accounts
such as mail, calendar, contacts, cloud storage, and photo providers.

This module currently exports shared data types only. It does not itself fetch accounts or
tokens, but it gives names to the values a GOA integration can return.

## Import

```aivi
use aivi.gnome.onlineAccounts (
    GoaAccountId
    GoaCapability
    GoaProvider
    GoaAccount
    AccessToken
    OAuthToken
    GoaMailAuth
    GoaMailAccount
    GoaError
    GoaAccountState
    GoaEvent
)
```

## Overview

| Type | Description |
|------|-------------|
| `GoaAccountId` | Text identifier for an account |
| `GoaCapability` | What an account can be used for |
| `GoaProvider` | Provider name as plain text |
| `GoaAccount` | Account record |
| `AccessToken` | Access token without refresh-token data |
| `OAuthToken` | OAuth token payload including optional refresh token |
| `GoaMailAuth` | Mail credential payload from GOA |
| `GoaMailAccount` | Mail-ready GOA account with IMAP/SMTP settings |
| `GoaError` | Account and credential failures |
| `GoaAccountState` | Whether the account is active, disabled, or needs attention |
| `GoaEvent` | Account add/remove/change events |

## Types

### GoaAccountId

```aivi
type GoaAccountId = Text
```

Plain-text identifier for a GNOME Online Accounts entry.

### GoaCapability

```aivi
type GoaCapability =
  | Mail
  | Calendar
  | Contacts
  | Files
  | Photos
```

Broad feature areas an account may support.

- `Mail` — email
- `Calendar` — calendar data
- `Contacts` — address book data
- `Files` — file storage or sync
- `Photos` — photo access or sync

### GoaProvider

```aivi
type GoaProvider = Text
```

Provider name as plain text. This stays flexible, so the module does not hard-code provider
names.

### GoaAccount

```aivi
type GoaAccount = {
    id: GoaAccountId
}
```

Account record.

Right now it is intentionally small: the only stored field is the account `id`.

```aivi
use aivi.gnome.onlineAccounts (GoaAccount)

value account : GoaAccount = {
    id: "personal-mail"
}
```

### AccessToken

```aivi
type AccessToken = {
    token: Text,
    tokenType: Text,
    expiresAt: Option Int
}
```

Simple access-token payload.

- `token` — the token value
- `tokenType` — the token kind, such as `"Bearer"`
- `expiresAt` — optional expiry time as an `Int`; this module does not define the unit

### OAuthToken

```aivi
type OAuthToken = {
    accessToken: Text,
    refreshToken: Option Text,
    tokenType: Text,
    expiresAt: Option Int
}
```

OAuth token payload with optional refresh-token support.

### GoaMailAuth

```aivi
type GoaMailAuth =
  | GoaMailPassword Text
  | GoaMailOAuthToken OAuthToken
```

Mail authentication material resolved from GOA for daemon-side use.

- `GoaMailPassword Text` — password-based IMAP/SMTP credential
- `GoaMailOAuthToken OAuthToken` — OAuth2 token payload suitable for XOAUTH2-style auth

### GoaError

```aivi
type GoaError =
  | AccountNotFound Text
  | CredentialUnavailable Text
  | AttentionRequired Text
  | GoaProtocolError Text
```

Common GOA failure cases.

- `AccountNotFound` — the requested account ID does not exist
- `CredentialUnavailable` — credentials could not be obtained
- `AttentionRequired` — the account needs user action before it can be used
- `GoaProtocolError` — another GOA/backend error reported as text

### GoaAccountState

```aivi
type GoaAccountState =
  | AccountActive
  | AccountNeedsAttention
  | AccountDisabled
```

High-level state of an account.

### GoaEvent

```aivi
type GoaEvent =
  | AccountAdded GoaAccountId
  | AccountRemoved GoaAccountId
  | AccountChanged GoaAccountId
```

Change notifications for account lists.

```aivi
use aivi.gnome.onlineAccounts (
    GoaEvent
    AccountAdded
    AccountRemoved
    AccountChanged
)

type GoaEvent -> Text
func describeEvent = event => event
 ||> AccountAdded id   -> "Added: {id}"
 ||> AccountRemoved id -> "Removed: {id}"
 ||> AccountChanged id -> "Changed: {id}"
```

### GoaMailAccount

```aivi
type GoaMailAccount = {
    id: GoaAccountId,
    provider: GoaProvider,
    providerType: Text,
    identity: Text,
    presentationIdentity: Text,
    state: GoaAccountState,
    emailAddress: Text,
    name: Text,
    imapHost: Text,
    imapPort: Int,
    imapUserName: Text,
    imapUseSsl: Bool,
    imapUseTls: Bool,
    smtpHost: Text,
    smtpPort: Int,
    smtpUserName: Text,
    smtpUseSsl: Bool,
    smtpUseTls: Bool,
    auth: GoaMailAuth
}
```

Mail-ready account record published by the current GOA source provider.

- Includes resolved endpoint settings for both IMAP and SMTP.
- `auth` carries either a password credential or an OAuth token payload.

## Source provider

```aivi
@source goa.mailAccounts
signal accounts : Signal (Result GoaError (List GoaMailAccount))
```

`goa.mailAccounts` publishes the current set of GOA mail accounts and republishes when GOA account
objects change on the session bus.
