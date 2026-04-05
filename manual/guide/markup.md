# Markup & UI

AIVI markup is an expression language for UI trees. It looks XML-like, but it is part of the language surface and participates in ordinary AIVI binding rules.

## Basic widgets

```aivi
value view =
    <Window title="Greeting">
        <Box>
            <Label text="Hello" />
            <Label text="World" />
        </Box>
    </Window>
```

Capitalised tags name widgets or components. Attributes can be plain text or embedded expressions.

## Binding expressions into attributes

```aivi
value header = "Users"

value view =
    <Window title="Directory">
        <Label text={header} />
    </Window>
```

Anything inside `{...}` is an AIVI expression.

## Typed widget properties

Widget attributes are schema-checked host properties, not an open CSS map. That means you bind ordinary expressions into the properties a widget exposes today.

```aivi
type Bool -> Float
func previewOpacity = .
 T|> 1.0
 F|> 0.25

value previewVisible = True

value view =
    <Window title="Preview">
        <Button label="Blue" animateOpacity={True} opacity={previewOpacity previewVisible} />
    </Window>
```

`Button.opacity` takes a `Float` and the GTK host clamps it into the `0.0` to `1.0` range. `Button.animateOpacity` takes a `Bool` and enables the built-in opacity transition class, so changes to `opacity` fade instead of snapping.

Because these are ordinary attributes, they can be driven by any signal-backed or source-backed expression. Arbitrary `cssProps` maps are not part of the current AIVI surface yet.

## Conditional rendering with `<show>`

```aivi
value isVisible = True

value view =
    <Window title="Status">
        <Box>
            <show when={isVisible} keepMounted={True}>
                <Label text="Visible" />
            </show>
        </Box>
    </Window>
```

`<show>` renders its body only when the condition holds.

## Local bindings with `<with>`

```aivi
type Item = {
    id: Int,
    title: Text
}

type Screen = Ready (List Item)

value screen =
    Ready [
        { id: 1, title: "Alpha" },
        { id: 2, title: "Beta" }
    ]

value view =
    <Window title="Items">
        <Box>
            <with value={screen} as={currentScreen}>
                <match on={currentScreen}>
                    <case pattern={Ready items}>
                        <each of={items} as={item} key={item.id}>
                            <Label text={item.title} />
                        </each>
                    </case>
                </match>
            </with>
        </Box>
    </Window>
```

`<with>` binds a value for the nested subtree.

## Pattern matching with `<match>`

Markup has control nodes for the same pattern-oriented style used elsewhere in the language:

```aivi
type Item = {
    id: Int,
    title: Text
}

type Screen =
  | Loading
  | Ready (List Item)
  | Failed Text

value screen =
    Ready [
        { id: 1, title: "Alpha" },
        { id: 2, title: "Beta" }
    ]

value view =
    <Window title="Items">
        <Box>
            <match on={screen}>
                <case pattern={Loading}>
                    <Label text="Loading..." />
                </case>
                <case pattern={Ready items}>
                    <each of={items} as={item} key={item.id}>
                        <Label text={item.title} />
                        <empty>
                            <Label text="No items" />
                        </empty>
                    </each>
                </case>
                <case pattern={Failed reason}>
                    <Label text={reason} />
                </case>
            </match>
        </Box>
    </Window>
```

## Iteration with `<each>`

`<each>` renders one subtree per list item and can include an `<empty>` fallback when the list is empty.

## Summary

| Element | Meaning |
| --- | --- |
| `<Label ... />` | A widget node |
| `<fragment>...</fragment>` | Group children without an outer widget |
| `<show when={...}>` | Conditional rendering |
| `<with value={...} as={...}>` | Bind a value in markup |
| `<match on={...}>` | Pattern-based rendering |
| `<case pattern={...}>` | One match branch |
| `<each of={...} as={...} key={...}>` | Iterate a list |
| `<empty>` | Empty-list fallback inside `<each>` |

---

**See also:** [Building Snake](building-snake.md) — a complete application that uses every markup feature together
