# Test fixtures

Most JSON here is **recorded from the old TypeScript app** (React UI in `src/web`, Node server in
`src/server`), so the Rust port can be checked against it field by field (plan Phase 0):

| Fixtures | Replayed by | Recorded with |
| --- | --- | --- |
| `logic/*.json` | `tests/logic_fixtures.rs` | `logic/generate.mjs` + `logic/loader.mjs` (ran `src/web/lib` under Node) |
| `fs/*.json` | `tests/fs_parity.rs` | `record-fs.mjs` (ran `src/server/fs` on `projects/`) |
| `git/expected.json` | `tests/git_integration.rs` | `git/record.ts` (ran `src/server/git.ts` on a repo from `git/make-repo.sh`, with the fake `git/gh`) |
| `llm/expected.json` | `tests/llm.rs` | `llm/record.ts` (ran `src/server/foundry.ts` against a local Azure-like server) |

The TypeScript app and the recorders were removed at the cutover (Phase 7). They are in git
history at commit `b578d2c`, which is also where the `src/…` files named in code comments live.
To re-record, check that commit out and run the recorder with Node 24 (see each script's header).

The fixtures are now the reference: when Rust behaviour changes on purpose, update the JSON with
the change and say why in the commit.

`projects/` holds the sample projects the fs fixtures were recorded on; symlinks and large files
are created at test time.
