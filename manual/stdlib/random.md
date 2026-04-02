# aivi.random

Randomness vocabulary plus the `RandomSource` capability-handle type.

Public randomness now goes through `@source random` handles rather than standalone imports.

## Import

```aivi
use aivi.random (
    RandomSource
    RandomError
    InsufficientEntropy
)
```

## Capability handle

```aivi
@source random
signal entropy : RandomSource

value dieRoll : Task Text Int = entropy.int 1 6
value csrfSeed : Task Text Bytes = entropy.bytes 32
value normalized : Task Text Float = entropy.float
```

## Exported vocabulary

- `RandomSource` - nominal handle annotation for `@source random`.
- `RandomError` / `InsufficientEntropy` - structured randomness failure vocabulary.

## Canonical handle members

| Member | Type | Description |
| --- | --- | --- |
| `entropy.int low high` | `Task Text Int` | Generate a whole number in the requested range |
| `entropy.bytes count` | `Task Text Bytes` | Generate random bytes |
| `entropy.float` | `Task Text Float` | Generate a random floating-point value |

The compiler still accepts longer compatibility spellings such as `randomInt`, `randomBytes`, and
`randomFloat` on the handle surface, but the docs prefer the shorter member names.
