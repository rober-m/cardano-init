# Releasing

Releases are built by [cargo-dist](https://opensource.axo.dev/cargo-dist/) (see
`.github/workflows/release.yml`) and triggered by pushing a version tag. The
release notes are sourced from `CHANGELOG.md`, which is generated from
[Conventional Commit](https://www.conventionalcommits.org/) history by
[git-cliff](https://git-cliff.org/) (see `cliff.toml`).

cargo-dist extracts the `CHANGELOG.md` section whose heading matches the tag and
places it at the **top** of the GitHub Release body, then appends its own
Install/Download tables below.

## Cutting a release

`main` is protected by a ruleset that requires changes to land via pull
request, so the release prep goes through a PR and the tag is created on the
merged `main` commit — **not** pushed alongside a branch. The changelog must be
regenerated and committed **before** tagging, because cargo-dist reads the
committed file at tag-push time.

```bash
# 1. Branch off an up-to-date main.
git checkout main && git pull --ff-only
git checkout -b chore/release-vX.Y.Z

# 2. Bump the version in Cargo.toml (`version = "X.Y.Z"`) and sync the lockfile.
cargo update -p cardano-init

# 3. Turn the [Unreleased] section into [vX.Y.Z] and refresh links.
#    A GitHub token MUST be in the environment or every "by @handle" is dropped
#    (see the attribution note below). git-cliff via nix; drop the
#    `nix run nixpkgs#` prefix if it's installed.
GITHUB_TOKEN="$(gh auth token)" \
  nix run nixpkgs#git-cliff -- --config cliff.toml --tag vX.Y.Z --output CHANGELOG.md

# 4. Review CHANGELOG.md, then commit and open a PR.
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "chore(release): vX.Y.Z"
git push -u origin chore/release-vX.Y.Z
gh pr create --title "chore(release): vX.Y.Z" --body "Release prep for vX.Y.Z."

# 5. After the PR is merged, tag the merged commit on main and push the tag.
#    Pushing the tag is what triggers the release workflow.
git checkout main && git pull --ff-only
git tag -a vX.Y.Z -m "vX.Y.Z"
git push origin vX.Y.Z
```

> Do **not** `git push origin main vX.Y.Z` to release: the branch push is
> rejected by the ruleset while the tag slips through, triggering a release from
> a commit that isn't on `main`. Always merge the PR first, then tag `main`.

## Notes

- Only `feat`, `fix`, `perf`, `refactor`, `docs`, and `revert` commits appear in
  the changelog. `test`/`chore`/`ci`/`build`/`style` and merge commits are
  filtered out (see the `commit_parsers` in `cliff.toml`).
- Commit-message hygiene drives note quality. A doc-only change committed as
  `feat(docs): …` will be grouped under **Features**, not **Documentation** —
  the first matching parser (`^feat`) wins.
- Contributor `@handles` are resolved locally during generation, so regenerate
  the changelog locally (not in CI) to keep attribution.
- **Attribution needs a GitHub token.** The `by @handle` suffix comes from
  git-cliff's GitHub API integration, which is *silently skipped* when no token
  is present — you get a valid-looking changelog with every `by @…` missing, and
  no error. Always run generation with a token in the environment, e.g.
  `GITHUB_TOKEN="$(gh auth token)"` as shown in step 3. `cliff.toml` pins the
  repo (`[remote.github]`) so resolution no longer depends on the branch's
  upstream being set, but it still cannot invent the token. A handful of legacy
  commits authored with a non-GitHub email (e.g. `…@<host>.local`) have no handle
  to resolve and stay unattributed — that is expected, not a token problem.
  If you spot missing `@handles` in a fresh section, regenerate with the token
  before opening the release PR; if a release already published without them,
  fix `CHANGELOG.md` in a follow-up PR and `gh release edit <tag> --notes-file`
  to correct the published body.
