# Domains

Domains wrap a representation type with a distinct surface: literals, operators, and named helpers belong to the domain instead of leaking raw carrier values everywhere.

## Declaring domains

```aivi
domain Duration over Int
    literal ms: Int -> Duration
    (+): Duration -> Duration -> Duration
    value: Duration -> Int

domain Path over Text
    literal root: Text -> Path
    (/): Path -> Text -> Path
    value: Path -> Text
```

## Parameterized domains are part of the syntax too

```aivi
domain NonEmpty A over List A
    fromList: List A -> Option (NonEmpty A)
    head: NonEmpty A -> A
    tail: NonEmpty A -> List A
```

## Bundled domains

The stdlib currently ships these domain families:

- `aivi.duration.Duration` with `ms`, `sec`, `min`, `millis`, `trySeconds`, `value`, `+`, and `-`
- `aivi.path.Path` with `parse`, `/`, and `value`
- `aivi.url.Url` with `parse` and `value`
- `aivi.color.Color` with `argb` and `value`
- `aivi.http.Retry` with the `x` literal used by backoff and HTTP retry options

```aivi
domain Duration over Int
    literal sec: Int -> Duration

domain Retry over Int
    literal x: Int -> Retry

val delay: Duration = 5sec
val retries: Retry = 3x
```

Domains are not just aliases. They introduce a separate typed surface over an underlying representation.
