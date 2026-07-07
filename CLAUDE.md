# ansidrama — working notes for Claude

## Session handover rule

**When your context window is more than ~25% filled, stop and hand over:**
write the current state to `docs/HANDOVER.md` (what's done, what's in flight,
the active branch, the next concrete step, and any pending decisions), then
stop so a fresh session can resume from that file. Read `docs/HANDOVER.md`
first at the start of a session if it exists.

## Build/test notes

- Cargo target dir is redirected to `/home/oetiker/scratch/cargo-target` — the
  release binary is at `.../release/ansidrama`, not `./target/`. Find it with
  `cargo metadata --format-version 1 | jq -r .target_directory`.
- Shared machine: cap parallelism to 4 cores — prefix cargo with
  `CARGO_BUILD_JOBS=4`.
- `record` is unix-only and needs no tmux (it embeds its own PTY+VT terminal).
