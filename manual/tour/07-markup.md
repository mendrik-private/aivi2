# Markup

GTK and libadwaita UI trees are written directly in AIVI markup. Widget tags are PascalCase; control nodes are lower-case reserved tags.

## Basic widgets

```aivi
val hero =
    <Window title="Milestone 1">
        <Box spacing={12}>
            <Label text="Frontend fixture corpus" />
            <Button label="Refresh" />
        </Box>
    </Window>
```

Attribute expressions use `{...}`. Text attributes support interpolation like `"Hello {name}"`.

## Control nodes

```aivi
type Item = {
    id: Int,
    title: Text
}

type Screen =
  | Loading
  | Ready (List Item)
  | Failed Text

fun itemLabel:Text item:Item =>
    item.title

val header = "Users"

val screen =
    Ready [
        { id: 1, title: "Alpha" },
        { id: 2, title: "Beta" }
    ]

val screenView =
    <fragment>
        <Label text={header} />
        <show when={True} keepMounted={True}>
            <with value={screen} as={currentScreen}>
                <match on={currentScreen}>
                    <case pattern={Loading}>
                        <Label text="Loading..." />
                    </case>
                    <case pattern={Ready items}>
                        <each of={items} as={item} key={item.id}>
                            <Label text={itemLabel item} />
                            <empty>
                                <Label text="No items" />
                            </empty>
                        </each>
                    </case>
                    <case pattern={Failed reason}>
                        <Label text="Error {reason}" />
                    </case>
                </match>
            </with>
        </show>
    </fragment>
```

The shipped control nodes are `<fragment>`, `<each>`, `<empty>`, `<show>`, `<with>`, `<match>`, and `<case>`.

Event attributes exist at the markup boundary, but keep examples grounded: handlers target bodyless input signals, not arbitrary statements.
