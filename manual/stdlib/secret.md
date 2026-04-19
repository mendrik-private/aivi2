# aivi.secret

Types for desktop secret storage backed by Secret Service / libsecret.

This module is shared vocabulary for `@source secret`.

## Import

```aivi
use aivi.secret (
    SecretSource
    SecretError
    SecretTask
    SecretUnavailable
    SecretLocked
    SecretCancelled
    SecretProtocolError
)
```

## Overview

| Type | Purpose |
| --- | --- |
| `SecretSource` | Handle annotation for `@source secret` |
| `SecretError` | Structured desktop keyring failures |
| `SecretTask A` | Background secret-storage work returning `A` |

## Capability handle

```aivi
use aivi.secret (SecretSource, SecretTask)

@source secret "io.mailfox"
signal secrets : SecretSource

value savedToken : SecretTask (Option Text) =
    secrets.lookup (Map {
        "account": "primary",
        "kind": "refresh-token"
    })
```

Current canonical handle members:

| Member | Type | Description |
| --- | --- | --- |
| `secrets.lookup attrs` | `SecretTask (Option Text)` | Look up one text secret in default desktop keyring collection |
| `secrets.store label attrs value` | `SecretTask Unit` | Store or replace one text secret in default desktop keyring collection |
| `secrets.delete attrs` | `SecretTask Bool` | Delete matching scoped secrets and report whether anything was removed |

The handle root argument scopes every operation. `@source secret "io.mailfox"` automatically adds
an internal service attribute so different apps can share user keyring safely without collisions.
