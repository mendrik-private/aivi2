# Introduction

AIVI is a purely functional, reactive language for GTK and libadwaita applications on Linux.

It is not Haskell-in-the-small or Elm-on-the-desktop. The current shipped surface is more conservative:

- declarations are explicit
- pipe algebra is primary
- signals are first-class
- GTK markup is native syntax
- effects enter through source-backed signals or `Task`s

## Mental model

`val` defines a stable value. `fun` defines a named function. `sig` defines a signal, either by derivation or through `@source`.

```aivi
fun step:Int tick:Unit current:Int =>
    current + 1

@source timer.every 120 with {
    immediate: True
}
sig tick: Signal Unit

sig counter: Signal Int =
    tick
     |> scan 0 step
```

`counter` is not mutated in place. It is derived by folding the `tick` signal through `step`.

## Markup is just another expression form

```aivi
type Status = Idle | Busy

fun statusLabel:Text status:Status =>
    status
     ||> Idle => "Idle"
     ||> Busy => "Busy"

val main =
    <Window title="Milestone 1">
        <Box spacing={12}>
            <Label text="Frontend fixture corpus" />
            <Label text={statusLabel Idle} />
        </Box>
    </Window>

export (statusLabel, main)
```

## What to avoid

When writing AIVI today, do **not** assume these surfaces exist:

- anonymous lambdas such as `x => x + 1`
- operator sections such as `_ + 5`
- pipe memo syntax such as `#name`
- wildcard imports
- `if/else` statements

Use named functions, pattern matching, projections such as `.field`, and the documented pipe operators instead.
