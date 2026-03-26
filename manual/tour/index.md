# Language Tour

This tour covers the shipped AIVI surface in the same order most programs use it:

1. values and types
2. functions
3. pipes
4. pattern matching
5. signals
6. sources
7. markup
8. type classes
9. domains

## A small complete program

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

Read the chapters as a reference, not as speculation. If you come from Haskell, Elm, F#, or PureScript, keep one rule in mind: only rely on the syntax you see implemented here.
