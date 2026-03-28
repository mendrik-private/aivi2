# Domains

A **domain** is a typed abstraction over a carrier type. It adds operators, literal syntax, and semantic behaviour to an existing type without changing the type itself.

Think of domains as a way to say: "I want `Int` to behave like a `Duration` in this context — with `s` suffixes and its own arithmetic rules."

## Declaring a Domain

```aivi
domain Duration over Int
```

This declares `Duration` as a domain over `Int`. Values of type `Duration` are integers at runtime, but the compiler tracks them as `Duration` and applies its operators.

## Literal Suffixes

Domains can define **literal suffixes** that make code read naturally:

```aivi
domain Duration over Int
    literal s: Int -> Duration

domain Retry over Int
    literal x: Int -> Retry
```

With these declarations, you can write:

```aivi
value timeout = 5s      -- Duration with value 5
value retries = 3x      -- Retry with value 3
```

The suffix is a zero-cost conversion — it just changes how the compiler categorises the value.

## Custom Operators

Domains can define operators that work specifically on their carrier type:

```aivi
domain Path over Text
    literal root: Text -> Path
    (/) : Path -> Text -> Path
    unwrap: Path -> Text
```

This gives `Path` values a `/` operator for path joining:

```aivi
value configDir: Path = root "/etc"
value configFile: Path = configDir / "app.conf"
```

## Domain Resolution

When you use a literal suffix or domain operator, the compiler resolves which domain applies. The resolution must be **unambiguous** — if two domains could apply to the same expression, the compiler reports an error.

This means domains are not implicit type classes — they are explicit, closed, and statically resolved.

## Built-In Domains

AIVI's source system uses domains for configuration values. For example, `http.get` options accept `Duration` and `Retry`:

```aivi
domain Duration over Int
    literal s: Int -> Duration

domain Retry over Int
    literal x: Int -> Retry

@source http.get "https://api.example.com/data" with {
    timeout: 10s,
    retry: 2x
}
signal data: Signal (Result HttpError Data)
```

## `NonEmpty` Domain

The standard library includes a `NonEmpty` domain for lists that are guaranteed to have at least one element:

```aivi
domain NonEmpty A over List A
    fromList: List A -> Option (NonEmpty A)
    head: NonEmpty A -> A
    tail: NonEmpty A -> List A
```

This prevents operations like "get the first element" from needing to return `Option`:

```aivi
fun firstItem: A xs: NonEmpty A =>
    head xs    -- always safe, no Option needed
```

## Summary

| Form | Purpose |
|---|---|
| `domain Name over BaseType` | Declare a domain |
| `literal suffix: BaseType -> Domain` | Add a literal suffix |
| `(op): Domain -> X -> Y` | Add an operator |
| `name: Domain -> Y` | Add a named operation |

Domains make numeric and text types safer and more expressive without adding runtime cost.
