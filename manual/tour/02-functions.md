# Functions

`fun` introduces a named function. Parameters are declared as `name:Type`, and the return type comes after the function name.

## Declaring and calling functions

```aivi
fun add:Int x:Int y:Int =>
    x + y

fun greet:Text name:Text =>
    "Hello {name}"

val total: Int = add 20 22
val greeting: Text = greet "Ada"
```

AIVI uses whitespace application. `add 20 22` means “call `add` with `20` and `22`”.

## Type inference

```aivi
fun addOne:Int value:Int =>
    value + 1

fun label value =>
    "Value {value}"

val next = addOne 41
val text = label next
```

## Named functions are first-class

```aivi
use aivi.order (min)

fun smaller:Bool left:Int right:Int =>
    left < right

val leastCount: Int = min smaller 4 2
```

## No anonymous lambda surface

The current shipped CST does **not** include anonymous functions such as `x => x + 1` or operator sections such as `_ + 5`.

When you need reusable behavior, name it with `fun`. When you just need to project a field inside a pipe, use ambient projection such as `.email`.
