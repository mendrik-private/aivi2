# Domains

Domains add typed behavior to an existing carrier type. They are how AIVI models things like durations, retries, paths, and other value families that should have their own operators or literal syntax.

## Declaring a domain

```aivi
domain Duration over Int

type Builder = Int -> Duration

domain Duration over Int
```

This declares a `Duration` domain whose runtime carrier is `Int`.

## Literal suffixes

A domain can define literal suffixes:

```aivi
domain Duration over Int

value delay : Duration = 250ms
```

Suffixes must be explicit and unambiguous. In current AIVI they must also be at least two characters long.

## Operators and named members

Domains can attach operators and named methods:

```aivi
domain Path over Text
```

That lets you write domain-aware expressions such as:

```aivi
domain Duration over Int

value total : Duration = 10ms + 5ms
value raw : Int = unwrap total
```

Callable members can also carry authored bodies. Declare the type first, then add an instance-style binding line:

```aivi
type Builder = Int -> Duration

domain Duration over Int
```

Inside the authored body, the current domain is implemented against its carrier representation. That means `build raw = raw` is valid for `Duration over Int`, while callers still see `build : Int -> Duration`.

## Generic domains

Domains can also be parameterised:

```aivi
domain NonEmpty A over List A
```

This is useful when you want stronger guarantees than the carrier type alone can express.

## Summary

| Form | Meaning |
| --- | --- |
| `domain Name over Carrier` | Declare a domain |
| `literal ms : Int -> Duration` | Add a literal suffix |
| `(+) : D -> D -> D` | Add an operator |
| `unwrap : D -> Carrier` | Add a named method |
| `member : T` + `member x = expr` | Add an authored callable member |
