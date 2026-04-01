# aivi.smtp

Types for outgoing mail configuration and messages.

SMTP is the protocol used to send email. This module defines the records an SMTP-backed feature can work with. The current stdlib file is data only: it does not send a message by itself.

## Import

```aivi
use aivi.smtp (
    Attachment
    SmtpConfig
    SmtpMessage
    SmtpError
    SmtpAuthFailed
    SmtpConnectionFailed
    RecipientRejected
    MessageTooLarge
    SmtpProtocolError
    SmtpTask
)
```

## Overview

| Type | Purpose |
|------|---------|
| `SmtpConfig` | Where to send mail |
| `Attachment` | One file attached to a message |
| `SmtpMessage` | Full outgoing message payload |
| `SmtpError` | Structured send failures |
| `SmtpTask` | Background send operation |

---

## `SmtpConfig`

```aivi
type SmtpConfig = {
    host: Text,
    port: Int
}
```

Connection details for an SMTP server.

```aivi
use aivi.smtp (SmtpConfig)

value mailServer : SmtpConfig = {
    host: "smtp.example.com",
    port: 587
}
```

---

## `Attachment`

```aivi
type Attachment = {
    filename: Text,
    contentType: Text,
    data: Bytes
}
```

One attached file.

- `filename` — the name shown to the recipient
- `contentType` — the MIME type for the attachment
- `data` — the raw attachment bytes

```aivi
use aivi.smtp (Attachment)

type Bytes -> Attachment
func pdfAttachment = data =>
    {
        filename: "invoice.pdf",
        contentType: "application/pdf",
        data: data
    }
```

---

## `SmtpMessage`

```aivi
type SmtpMessage = {
    from: Text,
    to: List Text,
    cc: List Text,
    bcc: List Text,
    subject: Text,
    bodyText: Text,
    bodyHtml: Option Text,
    attachments: List Attachment
}
```

A complete email message.

- `from` — sender address
- `to`, `cc`, `bcc` — recipient lists
- `subject` — message subject line
- `bodyText` — plain-text body
- `bodyHtml` — optional HTML version of the body
- `attachments` — files to include with the message

```aivi
use aivi.smtp (SmtpMessage)

value welcomeMessage : SmtpMessage = {
    from: "noreply@example.com",
    to: ["ada@example.com"],
    cc: [],
    bcc: [],
    subject: "Welcome",
    bodyText: "Thanks for signing up.",
    bodyHtml: None,
    attachments: []
}
```

---

## `SmtpError`

```aivi
type SmtpError =
  | SmtpAuthFailed
  | SmtpConnectionFailed Text
  | RecipientRejected Text
  | MessageTooLarge
  | SmtpProtocolError Text
```

Structured failure reasons for sending mail.

- `SmtpAuthFailed` — login failed
- `SmtpConnectionFailed Text` — the server could not be reached or the connection dropped
- `RecipientRejected Text` — a recipient address was rejected
- `MessageTooLarge` — the server refused the message size
- `SmtpProtocolError Text` — another SMTP-level failure occurred

```aivi
use aivi.smtp (
    SmtpError
    SmtpAuthFailed
    SmtpConnectionFailed
    RecipientRejected
    MessageTooLarge
    SmtpProtocolError
)

type SmtpError -> Text
func describeSmtpError = error => error
 ||> SmtpAuthFailed           -> "authentication failed"
 ||> SmtpConnectionFailed msg -> "connection failed: {msg}"
 ||> RecipientRejected addr   -> "recipient rejected: {addr}"
 ||> MessageTooLarge          -> "message too large"
 ||> SmtpProtocolError msg    -> "SMTP protocol error: {msg}"
```

---

## `SmtpTask`

```aivi
type SmtpTask = Task SmtpError Unit
```

Alias for a send operation that either completes successfully or fails with `SmtpError`. A successful SMTP task returns `Unit`, so the important outcome is completion rather than a payload.
