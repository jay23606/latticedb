# Format: LSL grammar v0

```
schema   := item*
item     := enum | facet | type
enum     := "enum" IDENT "{" IDENT ("," IDENT)* "}"
facet    := "facet" IDENT fields
type     := "type" IDENT ("=" IDENT ("+" IDENT)*)? fields
fields   := "{" (IDENT ":" ty "optional"? ","?)* "}"
ty       := "bool" | "int" ("in" INT (".." | "..=") INT)? | "text" ("max" INT)? | "bytes" ("max" INT)?
          | "ref" "<" IDENT ">" | "list" "<" ty ">" ("max" INT)? | IDENT   (enum name)
comment  := "//" to end of line
```

Checks: unique names; composed facets exist; fields unique across composed facets and own fields; refs
point at `type`s; enums non-empty and unique; ranges non-empty; `max` greater than 0; no nested lists.
Annotations (`@merge`, `@label`, ...) are parsed from v1.
