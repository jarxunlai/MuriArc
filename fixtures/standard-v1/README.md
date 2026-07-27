# standard-v1 synthetic dataset

This directory is the immutable, small, synthetic input shared by Server
acceptance and the Desktop E0001 fixture producer.

- `dataset.json` defines the domain records and fixed timeline.
- `manifest.json` pins expected counts and SHA-256 attachment digests.
- `schema.json` is the strict source schema.
- `files/` contains synthetic attachment payloads only.

Changing this baseline in place is prohibited after publication. A semantic
change requires a new named fixture generation and a new manifest digest.
No passwords, API keys, sessions, tokens, CSRF values, private keys, or real
animal/research data may be added here.
