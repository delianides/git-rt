# Changelog

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
