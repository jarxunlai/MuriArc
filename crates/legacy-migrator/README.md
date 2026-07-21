# muriarc-legacy-migrator

`muriarc-legacy-migrator` only reads a legacy MurisPro SQLite database. It never
updates the source file and never overwrites an existing MuriArc target or JSON
report.

```text
cargo run -p muriarc-legacy-migrator -- audit \
  --source /path/to/mice.db \
  --report audit.json

cargo run -p muriarc-legacy-migrator -- migrate \
  --source /path/to/mice.db \
  --target /path/to/new-muriarc.db \
  --report migration.json
```

Migration is deliberately one-way and previewable: run `audit` first, review
duplicate display identifiers, cage cached-count mismatches, and rejected
pedigree links, then run `migrate` against a path that does not exist.
