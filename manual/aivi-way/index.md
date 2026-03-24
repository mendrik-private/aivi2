# The AIVI Way

The Language Tour explained *what* AIVI's features are.
This section explains *how to use them together* — the patterns, idioms, and mental models
that experienced AIVI programmers reach for.

## The core pattern

Every AIVI program follows the same shape:

```
source events → signals → pure transformations → markup
```

1. **Sources** provide raw events: key presses, timer ticks, HTTP responses, button clicks.
2. **Signals** accumulate and transform those events into current application state.
3. **Pure functions** compute derived values from signal state.
4. **Markup** binds derived signals to GTK widgets.

The runtime wires all of this together. Your code is a pure description of the relationships.

## Think in transformations, not mutations

In most UI frameworks, you mutate state in response to events:

```text
// typical imperative approach (pseudo-code)
button.on('click', () => {
  this.count += 1
  this.label.text = `Clicked ${this.count} times`
})
```

In AIVI, you declare the relationships once:

```text
// TODO: add a verified AIVI example here
```

`labelText` is always `"Clicked {count} times"`. You do not update it. You declared it.

## Model everything as signal transformations

The rule of thumb: if a value can change, it is a signal. If it is derived from a signal,
it is also a signal. If it is constant, it is a `val`.

```text
// TODO: add a verified AIVI example here
```

## Keep functions pure

AIVI functions (`fun`) cannot have side effects. This is a feature, not a limitation.

- Functions are easy to test (no mocks, no setup).
- Functions are easy to reason about (the output depends only on the inputs).
- Functions can be reused across different signals.
- The compiler can optimize them freely.

All complexity lives in the signal graph, not inside functions.
Functions just describe *how to transform* a value.

## Sections in this chapter

| Section | Pattern |
|---|---|
| [Async Data](/aivi-way/async-data) | `@source http.get` → `Ok`/`Err` pipe chains |
| [Forms](/aivi-way/forms) | Per-field signals + `?\|>` gate + combined signal |
| [State](/aivi-way/state) | Local `sig` + shared top-level signals |
| [List Rendering](/aivi-way/list-rendering) | `<each>` + `*\|>` fan-out |
| [Error Handling](/aivi-way/error-handling) | `Ok`/`Err` as values, not exceptions |
