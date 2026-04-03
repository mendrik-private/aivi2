# Domains

A duration is not just an integer. A URL is not just a string. A file path is not just text. When you treat them as their raw types, mistakes happen: you pass milliseconds where seconds were expected, or a URL where a file path belongs.

Domains solve this by wrapping a **carrier type** with a **semantic name** and its own operations. The compiler prevents you from mixing them up.

```aivi
domain Duration over Int
domain Url over Text
domain Path over Text

value timeout : Duration = 5sec
value endpoint : Url = ...
value config : Path = ...
```

You cannot pass a `Duration` where a `Path` is expected, even though both are backed by primitive types.

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

## Block body syntax

When a domain has multiple members, group them inside `= { ... }`:

```aivi
domain Duration over Int = {
    literal ms  : Int -> Duration
    literal sec : Int -> Duration
    type Duration -> Duration -> Duration
    (+)
    type Duration -> Int
    unwrap
}
```

Each member follows the same rules as a standalone member line — the `= { ... }` form simply groups them together and makes the scope visual.

Authored bodies work inside blocks too:

```aivi
domain Snake over List Cell = {
    type List Cell -> Snake
    fromCells cells = cells

    type Snake -> Cell
    head snake = getOrElse (Cell 0 0) (listHead snake)

    type Snake -> Int
    length snake = listLength snake
}
```

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
| `domain Name over Carrier = { ... }` | Group domain members in a block |
