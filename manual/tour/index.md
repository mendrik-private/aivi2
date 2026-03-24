# Language Tour

This tour covers the AIVI language from first principles.
It assumes you can read code in any mainstream language but does not assume functional programming experience.

## Reading guide

Each chapter builds on the previous one. Read them in order on your first pass.

| Chapter | What you learn | Key concept |
|---|---|---|
| [01 · Values & Types](/tour/01-values-types) | `val`, `type`, sum and product types | Everything has a type; no null |
| [02 · Functions](/tour/02-functions) | `fun`, labeled parameters, calling conventions | Labeled params eliminate positional ambiguity |
| [03 · Pipes](/tour/03-pipes) | `\|>`, projection shorthand, chaining | Data flows left-to-right |
| [04 · Pattern Matching](/tour/04-pattern-matching) | `\|\|>`, exhaustiveness, nested patterns | Match shapes, not just values |
| [05 · Signals](/tour/05-signals) | `sig`, recurrence with `@\|>...<\|@` | Time-varying values |
| [06 · Sources](/tour/06-sources) | `@source`, `@recur.timer`, lifecycle | Where values come from |
| [07 · Markup](/tour/07-markup) | `<label>`, `<each>`, `<match>` | GTK widgets as AIVI expressions |
| [08 · Type Classes](/tour/08-typeclasses) | `class`, `instance`, `Eq`, `Show` | Interfaces with laws |

## A complete program

Before diving into the details, here is a small but complete AIVI program.
The tour will dissect each piece.

```text
// TODO: add a verified AIVI example here
```

Do not worry if parts are unfamiliar — the tour explains everything step by step.

[Start with Values & Types →](/tour/01-values-types)
