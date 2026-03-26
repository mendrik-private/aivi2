# The AIVI Way

The idiomatic shape of an AIVI program is:

1. define domain data and helpers with `type`, `domain`, `fun`, and `val`
2. bring in changing inputs with `@source` or explicit recurrence
3. derive more `sig`s with pipes, `scan`, and applicative clusters
4. render markup from the final data

```aivi
fun step:Int tick:Unit current:Int =>
    current + 1

@source timer.every 120 with {
    immediate: True
}
sig tick: Signal Unit

sig count: Signal Int =
    tick
     |> scan 0 step
```

The following chapters show concrete patterns for state, async data, errors, forms, and list rendering.
