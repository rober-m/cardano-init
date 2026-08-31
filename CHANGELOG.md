# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [v0.2.1] - 2026-08-20

### Documentation

- Keep v-prefix in changelog headings and document PR-based release flow (#73) by @rober-m
- Refresh README with a stronger hook and demo GIF (#76) by @rober-m

### Features

- Add --table for a compact tools-by-role matrix (#77) by @rober-m

### Fixes

- Detect tx3 despite @meshsdk dep, and silence git note (#74) by @rober-m

## [v0.2.0] - 2026-08-18

### Documentation

- Add installer-recipes CI badge to README by @rober-m
- Document the Gift Card reference example by @rober-m
- Correct inaccuracies vs implementation by @rober-m

### Features

- Add `fullstack` option that combines `on-chain` and `off-chain` roles in a single folder by @rober-m
- Implement Scalus on-chain, off-chain, and fullstack templates by @rober-m
- Add scalus to smoke test CI by @rober-m
- Add CI to verify dependency installers by @rober-m
- Clean pychache and allow running CI on selected PRs by @rober-m
- Add Plinth + restructure --nix generation to have composable flakes by @rober-m
- Make direnv allow non-interactive with a warning for non-trusted substituters by @rober-m
- Change validator to match default "Gift Card" example by @rober-m
- Gate experimental tools behind explicit opt-in (#11) by @rober-m
- Scalus templates use the Gift Card example, blueprint-driven off-chain by @rober-m
- Add Evolution SDK by @rober-m
- Data-driven off-chain ↔ provider compatibility by @rober-m
- Add Tx3 off-chain template (Gift Card) by @rober-m
- Remove unnecessary comments by @rober-m
- Generate AGENTS.md tailored to the selected stack by @rober-m
- Redesign CLI output and errors by @rober-m
- Add devnet smoke badge to README by @rober-m
- First draft on update/remove tooling by @rober-m
- Add git initialization, remove `cardano-init edit`, and improve help message by @rober-m
- `cardano-init add` replaces without prompt if git tree is clean by @rober-m
- Fold updating-project-ooling to main docs by @rober-m
- Fold updating-project-ooling to main docs by @rober-m
- Add more tests about updating tools functionality by @rober-m
- Fix network to preview, switch via CARDANO_NETWORK in .env by @rober-m
- Drop the web builder surface by @rober-m

### Fixes

- Don't crash installer verify on unstattable bin dir by @rober-m
- Correct broken doctor recipes and harden installer-recipes gate by @rober-m
- Detect tx3 off-chain (#58) by @rober-m
- Add FromStr impl for Network to fix clippy build by @rober-m
- Canonicalize nix_packages order (#65) by @rober-m

### Refactor

- Void datum + off-chain seed, dropping the redundant params by @rober-m

## [v0.1.0] - 2026-07-10

### Features

- Add `cardano-init doctor` by @rober-m
- Add flake.nix by @rober-m
- Add `cardano-init list` command by @rober-m
- Update README by @rober-m
- Add nix installer (closes #2) by @rober-m
- Improve install/usage instructions by @rober-m
- Update tests and docs by @rober-m
- Add cargo dist to hanble the releases
- Ignore .claude by @rober-m
- Added npm to publish platforms by @rober-m

### Fixes

- Remove redundant borrow in println (clippy 1.97) by @paulobressan

[v0.2.1]: https://github.com/input-output-hk/cardano-init/compare/v0.2.0..v0.2.1
[v0.2.0]: https://github.com/input-output-hk/cardano-init/compare/v0.1.0..v0.2.0
[v0.1.0]: https://github.com/input-output-hk/cardano-init/releases/tag/v0.1.0

