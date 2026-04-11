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

A domain can define integer suffix constructors:

```aivi
domain Score over Int
    suffix pts : Int = n => Score n

value highScore : Score = 9000pts
```

Suffixes must be explicit and unambiguous. In current AIVI they must also be at least two characters long.

## Operators and named members

Domains can attach operators and named methods directly under the declaration:

```aivi
domain Score over Int
    suffix pts : Int = n => Score n
    (+) : Score -> Score -> Score
    (+) = left right => Score (left.carrier + right.carrier)
```

That lets you write domain-aware expressions such as:

```aivi
value total : Score = 10pts + 5pts
value raw : Int = total.carrier
```

Callable members use the same two-line pattern: annotate the member, then bind it.

```aivi
domain Score over Int
    fromRaw : Int -> Score
    fromRaw = raw => Score raw
```

The body is checked against the carrier view of the domain, while callers still see the nominal signature.

## `self` and receiver-style members

When a member operates on the current domain value, you can write it in receiver style with `self`:

```aivi
domain Snake over List Cell
    fromCells : List Cell -> Snake
    fromCells = cells => Snake cells
    head : Cell
    head = getOrElse (Cell 0 0) (listHead self)
    length : Int
    length = listLength self
```

`fromCells` stays explicit because it constructs a `Snake` from a carrier value. `head` and `length` use `self`, so their receiver is implicit in the annotation.

## Generic domains

Domains can also be parameterised:

```aivi
domain NonEmpty A over List A
```

This is useful when you want stronger guarantees than the carrier type alone can express.

## The `.carrier` accessor

Every domain has a built-in `.carrier` accessor that returns the underlying carrier value at zero cost. You do not need to declare it — the compiler synthesizes it automatically:

```aivi
domain Score over Int
    suffix pts : Int = n => Score n

value raw : Int = (100pts).carrier
```

This is useful when you need to pass a domain value to a function that expects the carrier type:

```aivi
domain Snake over NonEmptyList Cell

value cells : List Cell = nelToList mySnake.carrier
```

Unlike user-defined domain members, `.carrier` is always available on every domain without any declaration.

## Summary

| Form | Meaning |
| --- | --- |
| `domain Name over Carrier` | Declare a domain |
| `suffix pts : Int = expr` | Add an integer suffix constructor |
| `(+) : D -> D -> D` + `(+) = x y => expr` | Add an operator |
| `member : T` + `member = x => expr` | Add an authored callable member |
| `self` | Implicit domain-typed receiver in authored bodies |
| `.carrier` | Built-in accessor returning the carrier value (always available) |
