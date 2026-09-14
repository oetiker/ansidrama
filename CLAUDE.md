# ansidrama — working notes for Claude

## Before starting work

- Always `git pull` before starting work so you're on the latest `main`.

## Build/test notes

- Cargo target dir is redirected to `/home/oetiker/scratch/cargo-target` — the
  release binary is at `.../release/ansidrama`, not `./target/`. Find it with
  `cargo metadata --format-version 1 | jq -r .target_directory`.
- Shared machine: cap parallelism to 4 cores — prefix cargo with
  `CARGO_BUILD_JOBS=4`.
- `record` is unix-only and needs no tmux (it embeds its own PTY+VT terminal).
