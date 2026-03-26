# List Rendering

Render lists with `<each>`, keep keys stable, and do collection reshaping in pure data code before you hit markup.

## Basic list rendering

```aivi
type Item = {
    id: Int,
    title: Text
}

fun itemLabel:Text item:Item =>
    item.title

val items = [
    { id: 1, title: "Alpha" },
    { id: 2, title: "Beta" }
]

val listView =
    <each of={items} as={item} key={item.id}>
        <Label text={itemLabel item} />
        <empty>
            <Label text="No items" />
        </empty>
    </each>
```

## Reshape first, render second

```aivi
use aivi.list (
    Partition
    partition
)

fun low:Bool value:Int =>
    value < 3

val split: (Partition Int) =
    partition low [
        1,
        3,
        2
    ]
```

## Fan out across carriers

```aivi
type User = {
    active: Bool,
    email: Text
}

fun joinEmails:Text items:List Text =>
    "joined"

val users: List User = [
    {
        active: True,
        email: "ada@example.com"
    }
]

val joinedEmails: Text =
    users
     *|> .email
     <|* joinEmails
```

The key rule is simple: transform collections in normal AIVI code, then keep markup focused on display.
