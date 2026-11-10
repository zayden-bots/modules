# Zayden modules

The feature crates of the Zayden Discord bot: one crate per feature (gambling,
levels, music, palworld, …), 27 in all. Serenity for Discord, Postgres via
`sqlx`. The bot binary lives in `bots`; routing traits, config and the runtime
live in `platform`; migrations live in `schema`.

## Layout

| Path                    | What it is                                                         |
| ----------------------- | ------------------------------------------------------------------ |
| `crates/<name>`         | one crate per feature, wired into the bot by the `bots` repo       |
| `docs/`                 | audits, `HONEYPOT.md`, `family_tree_graphics.md`                   |
| `.sqlx/`                | offline query cache; committed, regenerated on any query change    |
| `deny.toml`             | `cargo-deny` licences, advisories and allowed git sources          |
| `bacon.toml`            | watch jobs                                                         |
| `.github/workflows/`    | CI                                                                 |

Crates: `accounts`, `ai`, `deadlock`, `destiny2`, `family`, `gambling`,
`gold-star`, `greetings`, `honeypot`, `hosting`, `jellyfin`, `levels`, `lfg`,
`llamad2`, `marathon`, `misc`, `moderation`, `music`, `palworld`, `patreon`,
`reaction-roles`, `suggestions`, `temp-voice`, `ticket`, `verify`, `watch`,
`youtube`.

Module-to-module dependencies (from the crate manifests):

| Crate      | Depends on            |
| ---------- | --------------------- |
| `gambling` | `levels`              |
| `honeypot` | `moderation`          |
| `ticket`   | `ai`                  |
| `watch`    | `ai`, `jellyfin`      |

Every other crate depends only on the org crates below and external crates.

## Dependencies on other repos

| Repo       | Crates used                                   |
| ---------- | --------------------------------------------- |
| `platform` | `zayden-core`, `zayden-app`, `zayden-graphics` |
| `schema`   | `schema` (dev-dependency, for test migrations) |

They are git dependencies in the root `Cargo.toml`, tracking the default
branch; `Cargo.lock` pins the commit. All crates from one repo share one locked
commit. Never give two entries for the same org repo different
`branch`/`tag`/`rev` values: cargo would treat them as different crates and
types would stop matching. CI enforces it (step "One ref per org repository" in
the `deny` job).

### Taking an upstream change

Push upstream first, then in this repo:

```bash
cargo update schema zayden-app
```

Regenerate `.sqlx/` if queries changed (see Database), run the checks, commit
`Cargo.lock`. Downstream repos (`bots`, `dashboard`) then run
`cargo update schema zayden-app accounts` (one crate per org repo).

### Local cross-repo work without pushing

```bash
../infra/scripts/dev-link.sh on       # writes ../.cargo/config.toml with [patch] entries
../infra/scripts/dev-link.sh status
../infra/scripts/dev-link.sh off
```

A `Cargo.lock` written while the link is on must never be committed: run
`dev-link.sh off` and `cargo update <crate>` (or `git restore Cargo.lock`)
first.

Shared tooling files (`rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`,
`.cargo/config.toml`) are owned by `infra/template/`; run
`../infra/scripts/sync-standards.sh` to copy them, `--check` to report drift.
Lint tables in the root `Cargo.toml` are kept identical across repos by hand.

## Commands

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo fmt --all
```

`--all-targets` defaults to _off_ and silently skips tests, benches and
examples. Always pass it.

Clippy never codegens or links, so it reports green while the build is broken.
Only a real build exercises linking:

```bash
cargo build --workspace --all-targets
```

`bacon.toml` defines these as watch jobs:

```bash
bacon                    # clippy, the default job
bacon test               # cargo test
bacon fmt                # cargo fmt --all --check
bacon build              # link check
bacon check-offline      # cargo check with SQLX_OFFLINE=true
bacon nextest            # needs `cargo install cargo-nextest`
```

Keep the flags identical across this README, `bacon.toml` and CI. A local
command weaker than CI is how drift starts.

## Conventions

- Lints are declared once in the root `Cargo.toml` under `[workspace.lints]`;
  every crate opts in with `[lints] workspace = true`.
- Dependency versions are pinned once in `[workspace.dependencies]`; member
  crates say `foo.workspace = true` and may _add_ features on top, but never
  restate a version. Unused workspace entries cost nothing — cargo only
  resolves a dependency once a member opts in.
- `unwrap`/`expect`/`panic`/`todo`/`dbg!` are **denied** outside tests;
  `clippy.toml` re-permits them under `#[cfg(test)]`.
- Tests live in `tests/`, not in inline `#[cfg(test)] mod tests` blocks.
- Commands are registered and routed manually through Serenity traits. `poise`
  is not used.
- `rustfmt.toml` uses unstable options, so `rust-toolchain.toml` pins nightly
  for local work. Never write `cargo +nightly` — the prefix is redundant.

## CI

`.github/workflows/ci.yml` runs these jobs in parallel: `rustfmt`, workspace
`clippy`, workspace tests against a Postgres service, `cargo-deny`, and
`sqlx prepare --check`. The `deny` job also checks that each org repository
appears with a single ref. Everything except `rustfmt` builds on **stable**
(`RUSTUP_TOOLCHAIN=stable` outranks `rust-toolchain.toml`, and `RUSTFLAGS=""`
clears the nightly-only flags in `.cargo/config.toml`); `rustfmt` runs on
nightly.

`cargo deny check` allows git sources from the orgs listed under
`[sources.allow-org]` in `deny.toml`; any other git source needs an
`allow-git` entry.

## Database

Stand up a throwaway server and migrate it:

```bash
docker run --rm -d --name zayden-prepare -p 55432:5432 -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=zayden_prepare postgres:18-alpine
export DATABASE_URL="postgres://postgres:postgres@localhost:55432/zayden_prepare"
sqlx migrate run --source ../schema/migrations
```

With no sibling checkout, apply exactly what the lockfile pins (what CI does):

```bash
dir="$(cargo metadata --format-version 1 --locked \
  | jq -r '.packages[] | select(.name == "schema") | .manifest_path | rtrimstr("/Cargo.toml")')"
sqlx migrate run --source "$dir/migrations"
```

**Only ever run mutating `sqlx` commands against a throwaway database.** A
`DATABASE_URL` in a `.env` points at the live one; migrations there are the
user's call.

Tests that need a database use `#[sqlx::test(migrator = "schema::MIGRATOR")]`.
Migrations are embedded in the `schema` crate, so tests need only
`DATABASE_URL`.

`.sqlx/` must be committed and holds only this repo's queries. CI sets
`SQLX_OFFLINE=true`, so that cache is the only thing the query macros have to
check against. Regenerate it against an **empty, freshly-migrated** database —
`LEFT JOIN` nullability inference is plan-sensitive, and a cache built against
a populated DB will fail CI's `prepare --check`. The first command builds the
git dependencies offline so their queries do not leak into this repo's cache:

```bash
SQLX_OFFLINE=true cargo check --workspace --all-targets
cargo sqlx prepare --workspace -- --all-targets
```

If prepare fails: `git restore .sqlx`.
