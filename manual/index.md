---
layout: home

hero:
  name: "AIVI"
  text: "Reactive apps for Linux"
  tagline: A purely functional, GTK-first language that makes desktop software as composable as spreadsheet formulas.
  actions:
    - theme: brand
      text: Language Tour
      link: /tour/
    - theme: alt
      text: Introduction
      link: /introduction
    - theme: alt
      text: Playground
      link: /playground/

features:
  - icon: 🔁
    title: Signals, not callbacks
    details: Every value that changes over time is a signal. The runtime wires the dependency graph — you declare transformations.
  - icon: 🧩
    title: Pipe algebra
    details: Data flows left-to-right through typed pipes. Transform, gate, fan-out, and match — all as composable pipe operators.
  - icon: 🎨
    title: GTK / libadwaita first-class
    details: Markup tags like &lt;Window&gt;, &lt;Button&gt;, and &lt;each&gt; compile directly to native GTK4 widgets via the AIVI runtime.
  - icon: 🔒
    title: No null. No exceptions. No loops.
    details: Types are closed, exhaustive, and null-free. Control flow is pattern matching and recursion. Bugs hide nowhere to go.
---

## A taste of AIVI

This complete program renders a counter with increment and decrement buttons.

```text
-- declare a signal 'count' starting at 0
-- when the "increment" button is clicked, increase count by 1
-- when the "decrement" button is clicked, decrease count by 1
-- derive 'label' as the text representation of count
-- render a Window titled "Counter" containing a vertical Box
--   with a Label bound to label, a "+" Button, and a "−" Button
-- export main as the application entry point
```

`count` starts at `0`. Each `increment` or `decrement` event folds through `update`.
No event listeners. No mutable state. No async juggling.
