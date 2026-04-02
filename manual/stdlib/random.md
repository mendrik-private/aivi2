# aivi.random

Randomness vocabulary plus the `RandomSource` capability-handle type.

Public `randomBytes` / `randomInt` imports have been folded into `@source random`.

## Import

```aivi
use aivi.random (
    RandomSource
    RandomError
    InsufficientEntropy
    RandomTask
    RandomIntTask
    RandomBytesTask
    RandomFloatTask
)
```

## Capability handle

```aivi
@source random
signal entropy : RandomSource

value dieRoll : RandomIntTask = entropy.int 1 6
value csrfSeed : RandomBytesTask = entropy.bytes 32
value normalized : RandomFloatTask = entropy.float
```

## Exported vocabulary

- `RandomSource` — nominal handle annotation for `@source random`.
- `RandomError` — structured randomness failure vocabulary.
- `RandomTask`, `RandomIntTask`, `RandomBytesTask`, `RandomFloatTask` — current one-shot task
  aliases.

Like the other built-in handle families, one-shot value members still lower through the current
task intrinsic path, so these aliases use `Task Text ...`.
