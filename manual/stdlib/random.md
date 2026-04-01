# aivi.random

Random numbers and random bytes.

`aivi.random` gives you task-based access to host randomness. On the current Linux runtime,
the bytes come from `/dev/urandom`.

## Import

```aivi
use aivi.random (
    RandomError
    InsufficientEntropy
    RandomTask
    RandomIntTask
    RandomBytesTask
    randomBytes
    randomInt
)
```

## Overview

| Value | Type | Description |
| --- | --- | --- |
| `randomBytes` | `Int -> Task Text Bytes` | Read a requested number of random bytes |
| `randomInt` | `Int -> Int -> Task Text Int` | Pick a random integer in an inclusive range |
| `RandomError` | `InsufficientEntropy` | Structured random error type |
| `RandomTask` | `Task RandomError Bytes` | General random-byte task alias |
| `RandomIntTask` | `Task Text Int` | Task alias for integer generation |
| `RandomBytesTask` | `Task Text Bytes` | Task alias for byte generation |

## `randomBytes`

```aivi
randomBytes : Int -> Task Text Bytes
```

Request that many random bytes.

- `16` is a common size for a nonce or token seed
- `0` returns an empty byte buffer
- a negative count fails

```aivi
use aivi.random (randomBytes)

value sessionNonce : Task Text Bytes = randomBytes 16
```

## `randomInt`

```aivi
randomInt : Int -> Int -> Task Text Int
```

Pick a random integer from the inclusive range `low` to `high`.

If `low > high`, the task fails.

```aivi
use aivi.random (randomInt)

value dieRoll : Task Text Int = randomInt 1 6
value digit : Task Text Int = randomInt 0 9
```

## Error type

```aivi
type RandomError =
  | InsufficientEntropy
```

This is the module's structured error vocabulary.

**Current behavior note:** the callable functions `randomBytes` and `randomInt` currently
return `Task Text ...`, not `Task RandomError ...`. The `RandomError` and `RandomTask`
exports are still useful when you want your own API to name random-related failures.

## Example — generate two different kinds of randomness

```aivi
use aivi.random (
    randomBytes
    randomInt
)

value avatarColorIndex : Task Text Int = randomInt 0 7
value csrfSeed : Task Text Bytes = randomBytes 32
```
