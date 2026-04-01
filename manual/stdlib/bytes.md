# aivi.core.bytes

Byte sequence operations. All functions are runtime intrinsics — import them directly:

```aivi
use aivi.core.bytes (
    empty
    length
    get
    slice
    append
    fromText
    toText
    repeat
)
```

---

## Type

`Bytes` is a built-in immutable byte sequence. Individual bytes are `Int` values in the range `0–255`.

---

## Type

### `BytesDecodeError`

```aivi
type BytesDecodeError =
  | InvalidUtf8
```

Returned when `toText` fails because the byte sequence is not valid UTF-8.

---

## Intrinsics

### `empty : Bytes`

The empty byte sequence.

```aivi
use aivi.core.bytes (
    empty
    length
)
```

### `length : Bytes -> Int`

Number of bytes in the sequence.

```aivi
use aivi.core.bytes (
    fromText
    length
)
```

### `get : Int -> Bytes -> Option Int`

Return the byte at a zero-based index as an `Option Int` (0–255). Returns `None` when the index is out of bounds.

```aivi
use aivi.core.bytes (
    fromText
    get
)
```

### `slice : Int -> Int -> Bytes -> Bytes`

Return the sub-sequence from index `from` (inclusive) to `to` (exclusive). Out-of-range indices are clamped.

```aivi
use aivi.core.bytes (
    fromText
    slice
)
```

### `append : Bytes -> Bytes -> Bytes`

Concatenate two byte sequences.

```aivi
use aivi.core.bytes (
    fromText
    append
)
```

### `fromText : Text -> Bytes`

UTF-8 encode a `Text` value into `Bytes`.

```aivi
use aivi.core.bytes (
    fromText
    length
)
```

### `toText : Bytes -> Option Text`

UTF-8 decode `Bytes` into a `Text`. Returns `None` when the bytes are not valid UTF-8.

```aivi
use aivi.core.bytes (
    fromText
    toText
)
```

### `repeat : Int -> Int -> Bytes`

Create a byte sequence of `count` copies of a single byte value (0–255).

```aivi
use aivi.core.bytes (repeat)
```

---

## Real-world example

```aivi
use aivi.core.bytes (
    fromText
    toText
    length
    slice
    append
)

use aivi.fs (
    readBytes
    writeBytes
)

type Text -> Text -> Task Text Unit
func prependHeader = path header=>    let headerBytes
```

::: tip
Use `fromText`/`toText` for UTF-8 text round-trips. For binary data (images, archives, network frames) work directly with `Bytes` using `slice`, `append`, and `get`.
:::
