# Storage cleanup — 2026-10-19

This pass deleted two obsolete reference artifacts: the redundant release archive and superseded source compiler. Gross allocation removed: 29.65 MiB; net whole-checkout reduction: 29.6 MiB. No new parity builds or captures.

| Scope | Allocated GiB | Apparent GiB |
|---|---:|---:|
| Whole checkout | 11.54 | 22.636 |
| .reference | 6.494 | 16.644 |
| target | 2.865 | 3.766 |

Free space: 167.66 GiB. Every ceiling passes.

Deletion ledger: `.reference/cleanup-request-20261019/deletions-current-pass.jsonl`. Archive member verification, corpus integrity and fresh disk audit are in the same folder. All 67,356 current bindings and frozen sources verify. Selected retained executables already use LZFSE; no recompression or temporary backups added. The sole original trace archive remains because its replacement witness closure is unverified. Earlier cleanup ledger rows remain preserved.
