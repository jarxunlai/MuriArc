# MuriArc standard-v1 Desktop Fixture Producer

This internal, non-published tool converts the repository's strict synthetic
`fixtures/standard-v1` definition into a fresh Desktop SQLite `E0001` data
root. It calls Application use cases and the Store contract for all domain
writes; it does not copy an old SQLite database or relabel compatibility state
with ad-hoc SQL.

The same library is embedded in the Windows Desktop binary behind the explicit
`--muriarc-standard-fixture` maintenance switch. This lets RC tooling prove
that a fixture was produced by the exact final Desktop artifact.

The output path must either not exist or already contain a complete matching
receipt. Existing, partial, drifted, or symlinked data roots are rejected and
never cleared automatically.
