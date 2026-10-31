# Typed D2 records and field ownership

The adapter translates the Object and Edge records declared by the pinned Go
source, including labels, styles, tables, classes, references and icons. JSON
conversion follows typed defaults, case aliases, pointer merging and slice
reuse. Serialization emits canonical field names and Go’s omission rules.
Unknown record members are discarded; the graph’s `data` map retains arbitrary
application metadata.

## Layout fields

The engine updates object boxes, ordered edge routes, and supported label and
icon metadata. Those updates are serialized together with the other typed
record fields. A pointer to a zero value remains distinct from a nil pointer.
