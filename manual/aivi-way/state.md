# State

In AIVI, state lives in signal graphs and recurrence plans. Avoid thinking in terms of mutable boxes or component-local setters.

## Fold external wakeups into a signal

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

## Use explicit recurrence when the next step is scheduled

```aivi
domain Duration over Int
    literal s: Int -> Duration

type Cursor = { hasNext: Bool }

fun keep:Cursor cursor:Cursor =>
    cursor

val initial: Cursor = {
    hasNext: True
}

@recur.timer 1s
sig cursor: Signal Cursor =
    initial
     @|> keep
     ?|> .hasNext
     <|@ keep
```

Once you have a state signal, derive more signals from it. Do not treat `sig` as an imperative variable slot.
