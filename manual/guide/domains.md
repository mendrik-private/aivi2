# Domains

A score is not just an integer. A player ID is not just an integer either. When you treat them as their raw types, mistakes happen: you pass a score where a player ID was expected, or an ID where a count belongs.

Domains solve this by wrapping a **carrier type** with a **semantic name** and its own operations. The compiler prevents you from mixing them up.

```aivi
domain Score over Int

domain PlayerId over Int

domain Tag over Text

value highScore : Score = 9000
value currentPlayer : PlayerId = 7
value label : Tag = "featured"
```

You cannot pass a `Score` where a `PlayerId` is expected, even though both are backed by `Int`.

The standard library already ships [`Duration`](/stdlib/duration), [`Url`](/stdlib/url), and [`Path`](/stdlib/path) as built-in domains — you do not need to declare those yourself.

## Declaring a domain

```aivi
domain Score over Int
```

This declares a `Score` domain whose runtime carrier is `Int`.

## Literal suffixes

A domain can define literal suffixes:

```aivi
domain Score over Int = {
    literal pts : Int -> Score
}

value highScore : Score = 9000pts
```

Suffixes must be explicit and unambiguous. In current AIVI they must also be at least two characters long.

## Operators and named members

Domains can attach operators and named methods inside a block body:

```aivi
domain Score over Int = {
    literal pts : Int -> Score
    type Score -> Score -> Score
    (+)
    type Score -> Int
    unwrap
}
```

That lets you write domain-aware expressions such as:

```aivi
value total : Score = 10pts + 5pts
value raw : Int = unwrap total
```

Callable members can also carry authored bodies. Declare the type first, then add a binding line with the implementation:

```aivi
domain Score over Int = {
    type Int -> Score
    fromRaw raw = raw
}
```

Inside the authored body, the current domain is implemented against its carrier representation. That means `fromRaw raw = raw` is valid for `Score over Int`, while callers still see `fromRaw : Int -> Score`.

## Block body syntax

When a domain has multiple members, group them inside `= { ... }`:

```aivi
domain Score over Int = {
    literal pts : Int -> Score
    type Score -> Score -> Score
    (+)
    type Score -> Score -> Bool
    (<)
    type Score -> Int
    unwrap
}
```

Each member follows the same rules as a standalone member line — the `= { ... }` form simply groups them together and makes the scope visual.

Authored bodies work inside blocks too. Inside an authored body, `self` refers to the domain-typed receiver — its type is implicit and omitted from the annotation:

```aivi
domain Snake over List Cell = {
    type List Cell -> Snake
    fromCells cells = cells
    type Cell
    head = getOrElse (Cell 0 0) (listHead self)
    type Int
    length = listLength self
}
```

`fromCells` is a constructor — it takes a carrier value and wraps it. Since it does not use `self`, its annotation stays explicit. `head` and `length` operate on an existing `Snake`, so they use `self` and their annotations omit `Snake ->` from the first position.

## Generic domains

Domains can also be parameterised:

```aivi
domain NonEmpty A over List A
```

This is useful when you want stronger guarantees than the carrier type alone can express.

## The `.carrier` accessor

Every domain has a built-in `.carrier` accessor that returns the underlying carrier value at zero cost. You do not need to declare it — the compiler synthesizes it automatically:

```aivi
domain Score over Int = {
    literal pts : Int -> Score
}

value raw : Int = (100pts).carrier
```

This is useful when you need to pass a domain value to a function that expects the carrier type:

```aivi
domain Snake over NonEmptyList Cell

value cells : List Cell = nelToList mySnake.carrier
```

Unlike `unwrap`, which is a user-defined member you must declare yourself, `.carrier` is always available on every domain.

## Summary

| Form | Meaning |
| --- | --- |
| `domain Name over Carrier` | Declare a domain |
| `literal pts : Int -> Score` | Add a literal suffix |
| `(+) : D -> D -> D` | Add an operator |
| `unwrap : D -> Carrier` | Add a named method |
| `member : T` + `member x = expr` | Add an authored callable member |
| `self` | Implicit domain-typed receiver in authored bodies |
| `domain Name over Carrier = { ... }` | Group domain members in a block |
| `.carrier` | Built-in accessor returning the carrier value (always available) |
