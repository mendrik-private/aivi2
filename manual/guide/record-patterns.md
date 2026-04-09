# Record Patterns & Projections

Record patterns destructure values by field name. This page covers the full range of record destructuring forms, including dotted paths and projection expressions.

## Basic record patterns

Match a record and bind its fields to names:

```aivi
type Profile = {
    name: Text,
    score: Int
}

type Profile -> Text
func greet = .
 ||> { name } -> "Hello, {name}!"
```

You can bind multiple fields in one pattern:

```aivi
type Profile -> Text
func summary = .
 ||> { name, score } -> "{name} scored {score}"
```

See [Pattern Matching](/guide/pattern-matching) for full pattern syntax including tuples, constructors, and wildcards.

## Dotted path destructuring

When a record contains nested records, dotted paths reach into the structure without writing nested patterns manually:

```aivi
type City = {
    name: Text,
    population: Int
}

type Address = {
    city: City,
    street: Text
}

type User = {
    name: Text,
    address: Address
}

type User -> Text
func cityName = .
 ||> { address.city.name } -> name
```

`{ address.city.name }` is sugar for nested patterns:

```aivi
```

The leaf segment (`name`) becomes the bound variable. This works at any depth:

```aivi
type User -> Text
func streetName = .
 ||> { address.street } -> street
```

You can combine dotted paths with ordinary fields:

```aivi
type User -> Text
func userCity = .
 ||> { name, address.city.name: cityName } -> "{name} lives in {cityName}"
```

Here `address.city.name: cityName` renames the bound variable to `cityName` instead of the default leaf name.

## Record projection expressions

The `{ field: . }` form extracts a field and makes it the subject for further piping. The dot (`.`) means "this becomes the ambient subject":

```aivi
type Profile = {
    name: Text,
    score: Int
}

type Profile -> Bool
func isTopScore = .score >= 100
```

This is sugar for:

```aivi
type Profile -> Bool
func isTopScore = profile => profile
 ||> { score } -> score >= 100
```

The key insight: `{ field: . }` is not record construction — it is a **projection** that extracts the named field from the input.

## Dotted projection

Dotted paths combine with the projection form to reach into nested structures:

```aivi
type User -> Text
func getCityName = .address.city.name
```

This extracts `address.city.name` from the input and makes it available for downstream pipes:

```aivi
type User -> Text
func upperCityName = .address.city.name
  |> toUpperCase
```

The same dotted-path idea is also available in selected-subject function headers:

```aivi
type Z = { z: Int }

type Y = { y: Z }

type X = { x: Y }

type Int -> Int
func addOne = value =>
    value + 1

type X -> Int
func readNested = state { x.y.z! }
  |> addOne
```

Here `{ x.y.z! }` means "select `state.x.y.z` as the subject for the continuation." It is
projection sugar in the header, not a new general parameter-pattern form.

## Projection in pipes

Projection expressions work naturally as pipe stages:

```aivi
value uppercasedCity = user
  |> { address.city.name: . }
  |> toUpperCase
```

This is equivalent to the `.field` ambient projection form, but for deeper paths:

```aivi
value uppercasedCity = user
  |> .address
  |> .city
  |> .name
  |> toUpperCase
```

## Patch removal

The `: -` syntax in a patch removes a field from a record. The result type is the input type minus the removed field:

```aivi
type Full = {
    name: Text,
    email: Text,
    debug: Bool
}

type Full -> { name: Text, email: Text }
func stripDebug = .
```

Removal can target nested fields using selectors:

```aivi
```

See [Values & Functions § Structural patches](/guide/values-and-functions#structural-patches) for more on the `<|` operator and patch selectors.
