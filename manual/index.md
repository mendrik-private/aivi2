---
layout: home

hero:
  name: "AIVI"
  text: "Reactive GTK apps without callback soup"
  tagline: "A purely functional, GTK/libadwaita-first language built around pipe algebra, closed types, and source-backed signals."
  actions:
    - theme: brand
      text: Language Tour
      link: /tour/
    - theme: alt
      text: Introduction
      link: /introduction
    - theme: alt
      text: Standard Library
      link: /stdlib/

features:
  - icon: 🔁
    title: "Signals, not mutable state"
    details: "Values that change over time are explicit `Signal`s. You derive new signals; the scheduler handles propagation."
  - icon: 🧩
    title: "Pipe algebra"
    details: "`|>`, `?|>`, `||>`, `*|>`, `&|>`, `@|>`, and friends are the primary control-flow surface."
  - icon: 🎨
    title: "Native GTK markup"
    details: "Tags such as `<Window>`, `<Box>`, `<Label>`, and `<each>` compile to native GTK and libadwaita structures."
  - icon: 🔒
    title: "Conservative surface"
    details: "No null, no wildcard imports, no `if/else`, no loops, and no shipped anonymous-lambda syntax."
---

## A taste of AIVI

AIVI code is declaration-heavy: name your data, derive more data, and render GTK markup.

```aivi
val hero =
    <Window title="Milestone 1">
        <Box spacing={12}>
            <Label text="Frontend fixture corpus" />
            <Button label="Refresh" />
        </Box>
    </Window>
```

The rest of the manual stays close to the compiler and bundled stdlib. If a feature is not documented here or in `aivi.md`, assume it is not part of the stable surface yet.
