# Changelog

## [0.5.0](https://github.com/delianides/git-rt/compare/v0.4.0...v0.5.0) (2026-04-12)


### Features

* branch-scoped file list (merge base to worktree) ([#31](https://github.com/delianides/git-rt/issues/31)) ([4c8d182](https://github.com/delianides/git-rt/commit/4c8d18297ccfa9b1d9a2895ddb50301cd26a4f5a))

## [0.4.0](https://github.com/delianides/git-rt/compare/v0.3.0...v0.4.0) (2026-04-12)


### Features

* tab-based view with Changes, Commits, and PR tabs ([#25](https://github.com/delianides/git-rt/issues/25)) ([b7b1187](https://github.com/delianides/git-rt/commit/b7b1187f3c6f8737d8de1bfa54c47b8c306059f9))
* **ui:** remove the Commits tab ([#29](https://github.com/delianides/git-rt/issues/29)) ([e116db5](https://github.com/delianides/git-rt/commit/e116db54743b961817a9f821fd79949342fd873a))
* **ui:** remove the PR tab, add a compact PR status strip ([#30](https://github.com/delianides/git-rt/issues/30)) ([cd8f6fb](https://github.com/delianides/git-rt/commit/cd8f6fbfa0da468c6a52438137ced24cc612f878))
* **ui:** render tabs inside the main pane's top border ([#28](https://github.com/delianides/git-rt/issues/28)) ([0519843](https://github.com/delianides/git-rt/commit/0519843a4c847d2a2a62fb8c1455648531dde371))


### Bug Fixes

* **worktree:** activity filter, linked-gitdir paths, and PR poller restart ([#27](https://github.com/delianides/git-rt/issues/27)) ([65b668c](https://github.com/delianides/git-rt/commit/65b668c89a810fe2123f4e54d3f8e8e5025ec1fa))

## [0.3.0](https://github.com/delianides/git-rt/compare/v0.2.3...v0.3.0) (2026-04-10)


### Features

* cold-start auto-switch to most active worktree ([#23](https://github.com/delianides/git-rt/issues/23)) ([b950cc2](https://github.com/delianides/git-rt/commit/b950cc2e9baee254544b5d6ac13aafecc7a93b09))
* migrate git operations to gix and add transient error recovery ([#22](https://github.com/delianides/git-rt/issues/22)) ([6a48131](https://github.com/delianides/git-rt/commit/6a48131d15cc2fa204c9075439a849b6455cf981))
* theme file system with TOML/JSON support and --theme flag ([#21](https://github.com/delianides/git-rt/issues/21)) ([dac7bc4](https://github.com/delianides/git-rt/commit/dac7bc49fbc1bff6ea61c39e863395c4a316fb18))
* UI revamp with theme system and GitHub PR widget ([#19](https://github.com/delianides/git-rt/issues/19)) ([5f03ad2](https://github.com/delianides/git-rt/commit/5f03ad2205af21e8ebab235f4cfad823f0b0a1e8))
* **ui:** flash pane border on worktree switch ([b950cc2](https://github.com/delianides/git-rt/commit/b950cc2e9baee254544b5d6ac13aafecc7a93b09))

## [0.2.3](https://github.com/delianides/git-rt/compare/v0.2.2...v0.2.3) (2026-04-08)


### Bug Fixes

* revert auth header injection ([#17](https://github.com/delianides/git-rt/issues/17)) ([0a9351b](https://github.com/delianides/git-rt/commit/0a9351b01fa8517deb426dae2ce49d50c06d2adf))

## [0.2.2](https://github.com/delianides/git-rt/compare/v0.2.1...v0.2.2) (2026-04-08)


### Bug Fixes

* inject auth headers into Homebrew formula for private repo downloads ([#14](https://github.com/delianides/git-rt/issues/14)) ([0510995](https://github.com/delianides/git-rt/commit/0510995cc88a8e8abd2cfc3cdfdb018b76ca7276))

## [0.2.1](https://github.com/delianides/git-rt/compare/v0.2.0...v0.2.1) (2026-04-08)


### Bug Fixes

* use PAT for release-please to trigger tag-based workflows ([#12](https://github.com/delianides/git-rt/issues/12)) ([82d24c4](https://github.com/delianides/git-rt/commit/82d24c42023484e56296554a4a42f26f993fc5d5))

## [0.2.0](https://github.com/delianides/git-rt/compare/v0.1.0...v0.2.0) (2026-04-08)


### Features

* add cargo-dist for Homebrew and installer distribution ([#11](https://github.com/delianides/git-rt/issues/11)) ([73454ef](https://github.com/delianides/git-rt/commit/73454efb383ec8f41b6736c25ebe3a9e3cb08aa8))
* add right-align marker (%=) to file line format ([#4](https://github.com/delianides/git-rt/issues/4)) ([ce11773](https://github.com/delianides/git-rt/commit/ce1177343ed0f397715a4d214fed544d64f7e767))
* centralized color palette for statusbar format tags ([#10](https://github.com/delianides/git-rt/issues/10)) ([9b521f2](https://github.com/delianides/git-rt/commit/9b521f2e0944bb4560f3da9f4a29ddafacfa8160))
* configurable color theme support ([#5](https://github.com/delianides/git-rt/issues/5)) ([b6fd566](https://github.com/delianides/git-rt/commit/b6fd5664099315e34c53bda3a6481cf54320daab))
* configurable statusbar with format strings and inline colors ([#7](https://github.com/delianides/git-rt/issues/7)) ([27d2dd6](https://github.com/delianides/git-rt/commit/27d2dd6396f0d00a39b1e871dcb06bc641535845))
* independent top and bottom statusbars ([#8](https://github.com/delianides/git-rt/issues/8)) ([51c9ef1](https://github.com/delianides/git-rt/commit/51c9ef1a21a34044fa828a1c5327295755118047))
* worktree auto-follow and CLI pinning ([#9](https://github.com/delianides/git-rt/issues/9)) ([d923ef7](https://github.com/delianides/git-rt/commit/d923ef7f5a7bb90e49155ac09348f62cc5e86433))


### Bug Fixes

* chain release builds into release-please workflow ([d80a2f2](https://github.com/delianides/git-rt/commit/d80a2f2c5451ea1b7bc19d62b12b05736b50cd1d))
* stale file entries after commit and -0 +0 for staged files ([#6](https://github.com/delianides/git-rt/issues/6)) ([19f886f](https://github.com/delianides/git-rt/commit/19f886fde3db9aad53d6a18bd9c422dbc67ef74e))
* use macos-latest for x86_64-apple-darwin builds ([bf94029](https://github.com/delianides/git-rt/commit/bf940297e7d33015c17704261b41c853d3577482))

## 0.1.0 (2026-04-07)


### Features

* add nix flake, direnv, and rust-toolchain.toml for reproducible dev environment ([981bcb4](https://github.com/delianides/git-rt/commit/981bcb47857a463f21eded5c178cab48e99dab9c))
* configurable file line format ([#2](https://github.com/delianides/git-rt/issues/2)) ([60501a1](https://github.com/delianides/git-rt/commit/60501a1465c04d0efbd804c78bb06b3a8454e1b0))
