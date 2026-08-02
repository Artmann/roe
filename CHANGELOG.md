# Changelog

## [1.4.0](https://github.com/Artmann/roe/compare/roe-v1.3.1...roe-v1.4.0) (2026-08-02)


### Features

* **config:** Add per-analysis ignore globs for dupes, health, and dead-code ([3443072](https://github.com/Artmann/roe/commit/34430729a3deb61082ae9b97fe368b230bc53c71)), closes [#32](https://github.com/Artmann/roe/issues/32)
* **dupes:** Read thresholds and mode from the config file ([6c0053c](https://github.com/Artmann/roe/commit/6c0053c2e88ae542c888fffdc6b42103a56bcb45)), closes [#31](https://github.com/Artmann/roe/issues/31)

## [1.3.1](https://github.com/Artmann/roe/compare/roe-v1.3.0...roe-v1.3.1) (2026-07-27)


### Performance Improvements

* **check:** Share discovery and extraction across analyses ([b403ea4](https://github.com/Artmann/roe/commit/b403ea467e4cb4bffaeef63ff805d70fd86a8387))
* **dead-code:** Hash interned strings with FxHash instead of SipHash ([a1809fa](https://github.com/Artmann/roe/commit/a1809fa81de33a3111f5a4c059a0d7a92bb9eca6))
* **dead-code:** Resolve reference edges in parallel ([fe4f258](https://github.com/Artmann/roe/commit/fe4f2582562d38fb9004935f8d4337804e3d3962))
* **dead-code:** Reuse a scratch buffer when building candidate FQNs ([a0a175f](https://github.com/Artmann/roe/commit/a0a175f0cbbdc10863a8820004d92f35c871de50))

## [1.3.0](https://github.com/Artmann/roe/compare/roe-v1.2.0...roe-v1.3.0) (2026-07-26)


### Features

* **check:** Run all three analyses when no command is given ([8408a87](https://github.com/Artmann/roe/commit/8408a876c7271e17cabd1a28bb80d91bcae5c5ae))
* **health:** Add a baseline so CI can gate on new findings ([41f4267](https://github.com/Artmann/roe/commit/41f4267c76276845bf0bf0f43d9c6b27f09a0910)), closes [#23](https://github.com/Artmann/roe/issues/23)


### Bug Fixes

* **health:** Count required input parameters, not declared ones ([65e2a0e](https://github.com/Artmann/roe/commit/65e2a0e76922535271ad435bc61b2af91f93ed92))
* **health:** Exclude const fields from the large-type member count ([1f5b3cf](https://github.com/Artmann/roe/commit/1f5b3cf78c757fd8db6d27e69c9e908d0fbfbf31))
* **health:** Report what was actually scanned in the footer ([73a0be7](https://github.com/Artmann/roe/commit/73a0be7c09bd68a079f271bc322ea0e3c2de31ab))
* **health:** Stop counting ?? toward cyclomatic complexity ([bf82e47](https://github.com/Artmann/roe/commit/bf82e4710866e569c0b468f37162fa79465a253e))

## [1.2.0](https://github.com/Artmann/roe/compare/roe-v1.1.0...roe-v1.2.0) (2026-07-25)


### Features

* **health:** Add health command for complexity, size, coupling, and hotspots ([293ec96](https://github.com/Artmann/roe/commit/293ec96a6d8c7eac5a40240527690312ae328deb))
* **health:** Prioritize findings, honor suppressions, and read config ([f130555](https://github.com/Artmann/roe/commit/f130555c8aac6c76fb79d5fbb322c0d4c63e2e91))


### Bug Fixes

* **health:** Address PR review feedback ([5c22ecb](https://github.com/Artmann/roe/commit/5c22ecb9e22354013c6e320b33bf80019d6e56cd))

## [1.1.0](https://github.com/Artmann/roe/compare/roe-v1.0.2...roe-v1.1.0) (2026-07-23)


### Features

* **dead-code:** Add --library / libraryProjects to force per-project library mode ([b0b6ee7](https://github.com/Artmann/roe/commit/b0b6ee7a57bd6aa8bbbc8c57bdb235c789070f69)), closes [#8](https://github.com/Artmann/roe/issues/8)


### Bug Fixes

* **dead-code:** Don't flag compiler polyfill types as dead files ([23b6808](https://github.com/Artmann/roe/commit/23b68089a6889a3777aecfdd0dee14274dbadeaa)), closes [#7](https://github.com/Artmann/roe/issues/7)
* **dead-code:** Don't flag Deconstruct as an unused member ([84045e5](https://github.com/Artmann/roe/commit/84045e58c879d33ee5f1ef2ce03a7de9ad25ff12)), closes [#6](https://github.com/Artmann/roe/issues/6)
* **dead-code:** Flatten dotted member-access chains to resolve nested types ([f681396](https://github.com/Artmann/roe/commit/f681396b67d88f8c6174ce6c3b3a597bff874ddb)), closes [#5](https://github.com/Artmann/roe/issues/5)
* **release:** Apply the same always() guard to build and publish jobs ([b8f1a80](https://github.com/Artmann/roe/commit/b8f1a807a24386ba6e24f54e3ce81e9e8bebb583))
* **release:** Don't let an unrelated release-please failure skip test ([3d1d59b](https://github.com/Artmann/roe/commit/3d1d59b4f5d8cd479cc436baf303b1a383bc2882))

## [1.0.2](https://github.com/Artmann/roe/compare/roe-v1.0.1...roe-v1.0.2) (2026-07-23)


### Bug Fixes

* **release:** Publish to NuGet via trusted publishing instead of an API key ([7f2343a](https://github.com/Artmann/roe/commit/7f2343aeb2667a4b0331d22b17cd2d601fa8b3ad))

## [1.0.1](https://github.com/Artmann/roe/compare/roe-v1.0.0...roe-v1.0.1) (2026-07-23)


### Bug Fixes

* **release:** Trigger a release to validate the corrected tag parsing ([aff28c5](https://github.com/Artmann/roe/commit/aff28c5daf202f07a6427fa53b5e3c70ca271cd9))

## [1.0.0](https://github.com/Artmann/roe/compare/roe-v0.1.0...roe-v1.0.0) (2026-07-22)


### Features

* Distribute roe via NuGet, npm, and GitHub Releases ([0acfae3](https://github.com/Artmann/roe/commit/0acfae3c474bf29a2e4ab61cc6c58ca32e20cc95))
* **dupes:** Add duplicate code detection command ([89309ae](https://github.com/Artmann/roe/commit/89309ae968f0653e5fa53970de2c2c532f6722f9))
* **dupes:** Cut report noise with subsumption, line snapping, and impact ranking ([d7cd697](https://github.com/Artmann/roe/commit/d7cd69744b8c75c28b64026fbce872fdb4511346))
* **dupes:** Print duplicated source code, add --no-code to hide it ([f897f3f](https://github.com/Artmann/roe/commit/f897f3f1600cd7a73f7a9545250b4071e78994d5))
* Switch releasing to release-please ([6718bf9](https://github.com/Artmann/roe/commit/6718bf9f1e076f014abc481d83b1e8bc936e8cb4))


### Bug Fixes

* Make report paths stable across platforms ([be0f0d1](https://github.com/Artmann/roe/commit/be0f0d186a12d8fbf75e2b130999bcd014dcd4e9))
* Merge release-please into release.yml, drop PAT requirement ([b5b2238](https://github.com/Artmann/roe/commit/b5b2238bcbe676eb13c1558f27565ec60ad1e343))
* Normalize the unused-file finding name on Windows ([94b91bc](https://github.com/Artmann/roe/commit/94b91bca995f437d01d5345a883e6cccda7db48f))
