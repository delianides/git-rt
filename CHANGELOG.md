# Changelog

## [1.5.1](https://github.com/upsertco/perch/compare/v1.5.0...v1.5.1) (2026-05-21)


### Bug Fixes

* clarify branch base header ([#88](https://github.com/upsertco/perch/issues/88)) ([6cd8476](https://github.com/upsertco/perch/commit/6cd84761e1a995f0095f76037193211d05ab0bc7))
* Use strict default base branch resolution ([#86](https://github.com/upsertco/perch/issues/86)) ([c2216b2](https://github.com/upsertco/perch/commit/c2216b2bef4a64fa36a0f5a0f6a9a06b8ac8bb94))

## [1.5.0](https://github.com/upsertco/perch/compare/v1.4.0...v1.5.0) (2026-05-17)


### Features

* add Expanded view mode ([023b8a8](https://github.com/upsertco/perch/commit/023b8a832f73b705ef5042fe6cbeb492c13a6d36))

## [1.4.0](https://github.com/upsertco/perch/compare/v1.3.0...v1.4.0) (2026-05-17)


### Features

* reflog-aware base resolution ([#83](https://github.com/upsertco/perch/issues/83)) ([5e8ff7d](https://github.com/upsertco/perch/commit/5e8ff7d0f71990ebf25b509f421deb3061ed3fa7))
* rename git-rt to perch ([#81](https://github.com/upsertco/perch/issues/81)) ([7a97919](https://github.com/upsertco/perch/commit/7a979192055ed949d3b011e45b72cf9b687d888a))

## [1.3.0](https://github.com/upsertco/perch/compare/v1.2.0...v1.3.0) (2026-05-10)


### Features

* pinned-worktree mode (no auto-follow, trunk-only base, rename absorption) ([#80](https://github.com/upsertco/perch/issues/80)) ([53144e2](https://github.com/upsertco/perch/commit/53144e211e048cc0f65e28fa142e327c2967acf1))
* switch worktree dialog ([#75](https://github.com/upsertco/perch/issues/75)) ([8790b85](https://github.com/upsertco/perch/commit/8790b859c34e2a6816fbb9126c32a729ea984732))


### Bug Fixes

* **git:** pick closest merge-base when local and remote base diverge ([#78](https://github.com/upsertco/perch/issues/78)) ([9d09d36](https://github.com/upsertco/perch/commit/9d09d366a4b7aaa79f53a564881c9c4e12cb3843))
* **git:** use real git diff for overlay, replacing prefix/suffix synthesizer ([#76](https://github.com/upsertco/perch/issues/76)) ([0998dee](https://github.com/upsertco/perch/commit/0998dee3fef8cd6f7aba1ac2f40b08b091588171))


### Performance Improvements

* audit-driven hot-path improvements (recompute -50% on large repos) ([#79](https://github.com/upsertco/perch/issues/79)) ([0e8edba](https://github.com/upsertco/perch/commit/0e8edba26fe29c3b355cfdc8dfba554ba9f6f1f1))

## [1.2.0](https://github.com/upsertco/perch/compare/v1.1.0...v1.2.0) (2026-04-24)


### Features

* add tree view mode for file pane ([#72](https://github.com/upsertco/perch/issues/72)) ([88ba463](https://github.com/upsertco/perch/commit/88ba463d36da907bee59a5c41987be16b48032b6))
* **ui:** narrow-terminal adaptive rendering ([#74](https://github.com/upsertco/perch/issues/74)) ([88b2656](https://github.com/upsertco/perch/commit/88b265659f76a45187b8707fcfee1546036b5cf9))

## [1.1.0](https://github.com/upsertco/perch/compare/v1.0.5...v1.1.0) (2026-04-23)


### Features

* bring in-app diff modal back, remove pager path ([#71](https://github.com/upsertco/perch/issues/71)) ([ae9549a](https://github.com/upsertco/perch/commit/ae9549a55c9f9bbb1cb52255dae4cecd74aa71a1))
* **git:** per-branch base detection with reflog + merge-base heuristic ([#69](https://github.com/upsertco/perch/issues/69)) ([2df6820](https://github.com/upsertco/perch/commit/2df6820cbdcb99fa05b8e3a5b962d02d1f368758))
* vim-style scroll_padding for file list ([#67](https://github.com/upsertco/perch/issues/67)) ([779822d](https://github.com/upsertco/perch/commit/779822d3371306e3269edf4c01c5f0359e675fb7))


### Bug Fixes

* **state:** suppress row flash on initial file-list populate ([#70](https://github.com/upsertco/perch/issues/70)) ([cbd5cf2](https://github.com/upsertco/perch/commit/cbd5cf2babf955122a06874425969f2f71a96cd0))

## [1.0.5](https://github.com/upsertco/perch/compare/v1.0.4...v1.0.5) (2026-04-16)


### Bug Fixes

* **app:** pager flashes and closes on short diffs ([#66](https://github.com/upsertco/perch/issues/66)) ([932b755](https://github.com/upsertco/perch/commit/932b755e7a62194a356d2f49543724441a45649b))


### Performance Improvements

* **app:** async FsWatcher init + startup phase tracing ([#64](https://github.com/upsertco/perch/issues/64)) ([f1fb0a9](https://github.com/upsertco/perch/commit/f1fb0a93e96e7588304f28b51d2abb1294555bc6))

## [1.0.4](https://github.com/upsertco/perch/compare/v1.0.3...v1.0.4) (2026-04-16)


### Bug Fixes

* **docs:** remove references to removed in-app diff overlay and inline accordion ([#62](https://github.com/upsertco/perch/issues/62)) ([42b17df](https://github.com/upsertco/perch/commit/42b17dfcff243a45e0964502e941cfbcdaea34b7))

## [1.0.3](https://github.com/upsertco/perch/compare/v1.0.2...v1.0.3) (2026-04-16)


### Performance Improvements

* **git:** swap gix index-worktree walk for git CLI shell-out ([#58](https://github.com/upsertco/perch/issues/58)) ([ef96923](https://github.com/upsertco/perch/commit/ef96923a6475656cb9143fd22556ca40ba011cdd))

## [1.0.2](https://github.com/upsertco/perch/compare/v1.0.1...v1.0.2) (2026-04-15)


### Bug Fixes

* **ui:** force full clear on overlay dismissal to prevent stale cells ([#53](https://github.com/upsertco/perch/issues/53)) ([815c080](https://github.com/upsertco/perch/commit/815c08013ca3ba40586cbe7a09f5af93175caae5))


### Performance Improvements

* **app:** move git operations to a worker thread ([#56](https://github.com/upsertco/perch/issues/56)) ([a0e6db3](https://github.com/upsertco/perch/commit/a0e6db3af7e358d3d2bbd3db3b76ce8237f728b1))
* **git:** branch_status via tree-diff (O(changed) not O(all)) ([#57](https://github.com/upsertco/perch/issues/57)) ([65d71db](https://github.com/upsertco/perch/commit/65d71db7da2928afd01678850959d72b363c78aa))
* **watcher:** filter ignored dirs + raise debounce/tick defaults ([#55](https://github.com/upsertco/perch/issues/55)) ([0106736](https://github.com/upsertco/perch/commit/010673639a44f22a17a0581469852caa8a7d8faf))

## [1.0.1](https://github.com/upsertco/perch/compare/v1.0.0...v1.0.1) (2026-04-14)


### Bug Fixes

* **cli:** discover repo root so perch works from any subdirectory ([#51](https://github.com/upsertco/perch/issues/51)) ([815fda5](https://github.com/upsertco/perch/commit/815fda5b9999912739e558040361cd88d751d0bf))

## [1.0.0](https://github.com/upsertco/perch/compare/v0.8.0...v1.0.0) (2026-04-14)


### ⚠ BREAKING CHANGES

* **app:** use git pager for `d` key instead of difftool ([#50](https://github.com/upsertco/perch/issues/50))
* **cli:** drop --worktree; --branch searches all worktrees ([#48](https://github.com/upsertco/perch/issues/48))

### Features

* **app:** use git pager for `d` key instead of difftool ([#50](https://github.com/upsertco/perch/issues/50)) ([af138ca](https://github.com/upsertco/perch/commit/af138cac28b15924fd135ffa3cc0baab7edce361))
* **cli:** drop --worktree; --branch searches all worktrees ([#48](https://github.com/upsertco/perch/issues/48)) ([2021b60](https://github.com/upsertco/perch/commit/2021b60c6f66212dcb8702cdbc4607edf1cfccd1))

## [0.8.0](https://github.com/upsertco/perch/compare/v0.7.0...v0.8.0) (2026-04-13)


### Features

* **app:** open detected PR in browser on `p` ([#46](https://github.com/upsertco/perch/issues/46)) ([69466c9](https://github.com/upsertco/perch/commit/69466c95f25497f386e53e02b393969b996e46c8))
* **app:** open selected file in configured git difftool on `d` ([#47](https://github.com/upsertco/perch/issues/47)) ([b81d13d](https://github.com/upsertco/perch/commit/b81d13ddb5a6adba6147717ab51554183b372043))
* edit selected file on 'e' ([#45](https://github.com/upsertco/perch/issues/45)) ([d8c39eb](https://github.com/upsertco/perch/commit/d8c39eb0ddc769e04ddb7fdb76e710063963d1d4))
* remove actions system ([#43](https://github.com/upsertco/perch/issues/43)) ([a0de06b](https://github.com/upsertco/perch/commit/a0de06b771c96fc35a2e4cc44086981ca6b9c748))


### Bug Fixes

* **activity:** rank worktrees by git-native signals ([#42](https://github.com/upsertco/perch/issues/42)) ([11a264c](https://github.com/upsertco/perch/commit/11a264c64a2744452e976fe7d682331b11b5d1c5))

## [0.7.0](https://github.com/upsertco/perch/compare/v0.6.0...v0.7.0) (2026-04-13)


### Features

* fall back to main worktree when watched tree is removed ([#41](https://github.com/upsertco/perch/issues/41)) ([c2c2b98](https://github.com/upsertco/perch/commit/c2c2b98cbe42b9091d4ade21d2be1b9029c3194d))
* **ui:** move repo/branch into header, hide bottom bar when no PR ([#39](https://github.com/upsertco/perch/issues/39)) ([65dc73a](https://github.com/upsertco/perch/commit/65dc73abb59823455ba2b49ca11fbec57b563de7))

## [0.6.0](https://github.com/upsertco/perch/compare/v0.5.0...v0.6.0) (2026-04-12)


### Features

* **github:** show merged PR state instead of clearing it ([#36](https://github.com/upsertco/perch/issues/36)) ([4dbd678](https://github.com/upsertco/perch/commit/4dbd67818e67d54fb68c6c4d91df8a257735bc27))
* help popup (?) and spacebar diff toggle ([#38](https://github.com/upsertco/perch/issues/38)) ([8304591](https://github.com/upsertco/perch/commit/8304591f7a941b3a7d0620e6eba4f79d3fef7fea))
* replace activity-based worktree switching with branch-change detection ([#35](https://github.com/upsertco/perch/issues/35)) ([fd0d548](https://github.com/upsertco/perch/commit/fd0d548a5e7e1eb23eb7bdacf5159005735e81a0))
* **theme:** dedicated status marker colors ([#33](https://github.com/upsertco/perch/issues/33)) ([dc04fdd](https://github.com/upsertco/perch/commit/dc04fddf0c53752989c96b77db1d88b447fefd48))
* **ui:** hybrid checks display ([#37](https://github.com/upsertco/perch/issues/37)) ([af0d2e7](https://github.com/upsertco/perch/commit/af0d2e771dea46b2cac838605fa7f63d7feb92c0))

## [0.5.0](https://github.com/upsertco/perch/compare/v0.4.0...v0.5.0) (2026-04-12)


### Features

* branch-scoped file list (merge base to worktree) ([#31](https://github.com/upsertco/perch/issues/31)) ([4c8d182](https://github.com/upsertco/perch/commit/4c8d18297ccfa9b1d9a2895ddb50301cd26a4f5a))

## [0.4.0](https://github.com/upsertco/perch/compare/v0.3.0...v0.4.0) (2026-04-12)


### Features

* tab-based view with Changes, Commits, and PR tabs ([#25](https://github.com/upsertco/perch/issues/25)) ([b7b1187](https://github.com/upsertco/perch/commit/b7b1187f3c6f8737d8de1bfa54c47b8c306059f9))
* **ui:** remove the Commits tab ([#29](https://github.com/upsertco/perch/issues/29)) ([e116db5](https://github.com/upsertco/perch/commit/e116db54743b961817a9f821fd79949342fd873a))
* **ui:** remove the PR tab, add a compact PR status strip ([#30](https://github.com/upsertco/perch/issues/30)) ([cd8f6fb](https://github.com/upsertco/perch/commit/cd8f6fbfa0da468c6a52438137ced24cc612f878))
* **ui:** render tabs inside the main pane's top border ([#28](https://github.com/upsertco/perch/issues/28)) ([0519843](https://github.com/upsertco/perch/commit/0519843a4c847d2a2a62fb8c1455648531dde371))


### Bug Fixes

* **worktree:** activity filter, linked-gitdir paths, and PR poller restart ([#27](https://github.com/upsertco/perch/issues/27)) ([65b668c](https://github.com/upsertco/perch/commit/65b668c89a810fe2123f4e54d3f8e8e5025ec1fa))

## [0.3.0](https://github.com/upsertco/perch/compare/v0.2.3...v0.3.0) (2026-04-10)


### Features

* cold-start auto-switch to most active worktree ([#23](https://github.com/upsertco/perch/issues/23)) ([b950cc2](https://github.com/upsertco/perch/commit/b950cc2e9baee254544b5d6ac13aafecc7a93b09))
* migrate git operations to gix and add transient error recovery ([#22](https://github.com/upsertco/perch/issues/22)) ([6a48131](https://github.com/upsertco/perch/commit/6a48131d15cc2fa204c9075439a849b6455cf981))
* theme file system with TOML/JSON support and --theme flag ([#21](https://github.com/upsertco/perch/issues/21)) ([dac7bc4](https://github.com/upsertco/perch/commit/dac7bc49fbc1bff6ea61c39e863395c4a316fb18))
* UI revamp with theme system and GitHub PR widget ([#19](https://github.com/upsertco/perch/issues/19)) ([5f03ad2](https://github.com/upsertco/perch/commit/5f03ad2205af21e8ebab235f4cfad823f0b0a1e8))
* **ui:** flash pane border on worktree switch ([b950cc2](https://github.com/upsertco/perch/commit/b950cc2e9baee254544b5d6ac13aafecc7a93b09))

## [0.2.3](https://github.com/upsertco/perch/compare/v0.2.2...v0.2.3) (2026-04-08)


### Bug Fixes

* revert auth header injection ([#17](https://github.com/upsertco/perch/issues/17)) ([0a9351b](https://github.com/upsertco/perch/commit/0a9351b01fa8517deb426dae2ce49d50c06d2adf))

## [0.2.2](https://github.com/upsertco/perch/compare/v0.2.1...v0.2.2) (2026-04-08)


### Bug Fixes

* inject auth headers into Homebrew formula for private repo downloads ([#14](https://github.com/upsertco/perch/issues/14)) ([0510995](https://github.com/upsertco/perch/commit/0510995cc88a8e8abd2cfc3cdfdb018b76ca7276))

## [0.2.1](https://github.com/upsertco/perch/compare/v0.2.0...v0.2.1) (2026-04-08)


### Bug Fixes

* use PAT for release-please to trigger tag-based workflows ([#12](https://github.com/upsertco/perch/issues/12)) ([82d24c4](https://github.com/upsertco/perch/commit/82d24c42023484e56296554a4a42f26f993fc5d5))

## [0.2.0](https://github.com/upsertco/perch/compare/v0.1.0...v0.2.0) (2026-04-08)


### Features

* add cargo-dist for Homebrew and installer distribution ([#11](https://github.com/upsertco/perch/issues/11)) ([73454ef](https://github.com/upsertco/perch/commit/73454efb383ec8f41b6736c25ebe3a9e3cb08aa8))
* add right-align marker (%=) to file line format ([#4](https://github.com/upsertco/perch/issues/4)) ([ce11773](https://github.com/upsertco/perch/commit/ce1177343ed0f397715a4d214fed544d64f7e767))
* centralized color palette for statusbar format tags ([#10](https://github.com/upsertco/perch/issues/10)) ([9b521f2](https://github.com/upsertco/perch/commit/9b521f2e0944bb4560f3da9f4a29ddafacfa8160))
* configurable color theme support ([#5](https://github.com/upsertco/perch/issues/5)) ([b6fd566](https://github.com/upsertco/perch/commit/b6fd5664099315e34c53bda3a6481cf54320daab))
* configurable statusbar with format strings and inline colors ([#7](https://github.com/upsertco/perch/issues/7)) ([27d2dd6](https://github.com/upsertco/perch/commit/27d2dd6396f0d00a39b1e871dcb06bc641535845))
* independent top and bottom statusbars ([#8](https://github.com/upsertco/perch/issues/8)) ([51c9ef1](https://github.com/upsertco/perch/commit/51c9ef1a21a34044fa828a1c5327295755118047))
* worktree auto-follow and CLI pinning ([#9](https://github.com/upsertco/perch/issues/9)) ([d923ef7](https://github.com/upsertco/perch/commit/d923ef7f5a7bb90e49155ac09348f62cc5e86433))


### Bug Fixes

* chain release builds into release-please workflow ([d80a2f2](https://github.com/upsertco/perch/commit/d80a2f2c5451ea1b7bc19d62b12b05736b50cd1d))
* stale file entries after commit and -0 +0 for staged files ([#6](https://github.com/upsertco/perch/issues/6)) ([19f886f](https://github.com/upsertco/perch/commit/19f886fde3db9aad53d6a18bd9c422dbc67ef74e))
* use macos-latest for x86_64-apple-darwin builds ([bf94029](https://github.com/upsertco/perch/commit/bf940297e7d33015c17704261b41c853d3577482))

## 0.1.0 (2026-04-07)


### Features

* add nix flake, direnv, and rust-toolchain.toml for reproducible dev environment ([981bcb4](https://github.com/upsertco/perch/commit/981bcb47857a463f21eded5c178cab48e99dab9c))
* configurable file line format ([#2](https://github.com/upsertco/perch/issues/2)) ([60501a1](https://github.com/upsertco/perch/commit/60501a1465c04d0efbd804c78bb06b3a8454e1b0))
