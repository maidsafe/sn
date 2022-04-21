# Changelog

All notable changes to this project will be documented in this file. See [standard-version](https://github.com/conventional-changelog/standard-version) for commit guidelines.

## [0.60.0](https://github.com/maidsafe/sn_client/compare/v0.59.3...v0.60.0) (2021-06-11)


### ⚠ BREAKING CHANGES

* sn_messaging bump to 35

### Features

* update sn_messaging ([1656625](https://github.com/maidsafe/sn_client/commit/165662508532ec343bcf367e53a8f4b1f54d128e))

### [0.59.3](https://github.com/maidsafe/sn_client/compare/v0.59.2...v0.59.3) (2021-06-10)

### [0.59.2](https://github.com/maidsafe/sn_client/compare/v0.59.1...v0.59.2) (2021-06-10)


### Features

* discard blob errors if we get a positive one instead ([cc131a2](https://github.com/maidsafe/sn_client/commit/cc131a22e1d9cb177c6cd598810a50b22ade65be))

### [0.59.1](https://github.com/maidsafe/sn_client/compare/v0.59.0...v0.59.1) (2021-06-10)


### Features

* always reconnect to elders ([1a3afa3](https://github.com/maidsafe/sn_client/commit/1a3afa3117d5f44036b48bf6799f2695cc3dfd78))

## [0.59.0](https://github.com/maidsafe/sn_client/compare/v0.58.0...v0.59.0) (2021-06-09)


### ⚠ BREAKING CHANGES

* sn_messaging bump non-backward compatible.

* changes to upgrade sn_messaging to v34.0 ([3d405a7](https://github.com/maidsafe/sn_client/commit/3d405a73bf555e0d9aef32ae71c3cd92d322e52a))

## [0.58.0](https://github.com/maidsafe/sn_client/compare/v0.57.5...v0.58.0) (2021-06-08)


### ⚠ BREAKING CHANGES

* sn_messaging bump

### Features

* update sn_messaging ([458ec64](https://github.com/maidsafe/sn_client/commit/458ec6471fd2e962e0b6b55679d92e048bc212fc))


### Bug Fixes

* handle history failure more properly ([f46f025](https://github.com/maidsafe/sn_client/commit/f46f025a79f7ac5fb0f5e9baf13e69fea110aebf))

### [0.57.5](https://github.com/maidsafe/sn_client/compare/v0.57.4...v0.57.5) (2021-06-07)

### [0.57.4](https://github.com/maidsafe/sn_client/compare/v0.57.3...v0.57.4) (2021-06-05)

### [0.57.3](https://github.com/maidsafe/sn_client/compare/v0.57.2...v0.57.3) (2021-06-04)


### Bug Fixes

* **messaging:** don't discard error responses for Blob Queries ([c81a35a](https://github.com/maidsafe/sn_client/commit/c81a35abe96de7b397a8cc3780e6870bd0a98c33))

### [0.57.2](https://github.com/maidsafe/sn_client/compare/v0.57.1...v0.57.2) (2021-06-03)


### Bug Fixes

* **query:** timeout when we don't get a response for a query ([9e6b782](https://github.com/maidsafe/sn_client/commit/9e6b7827f8cd054200a61887bc20c33e634b93bb))

### [0.57.1](https://github.com/maidsafe/sn_client/compare/v0.57.0...v0.57.1) (2021-06-03)


### Features

* add payment buffer for storecost fluctuations ([a4eecfa](https://github.com/maidsafe/sn_client/commit/a4eecfa035dca6b6c39e131f36b5204be6e7c0c6))

## [0.57.0](https://github.com/maidsafe/sn_client/compare/v0.56.1...v0.57.0) (2021-06-02)


### ⚠ BREAKING CHANGES

* **messaging:** sn_messaging updated

Also expands some logging

### Features

* **messaging:** Update sn_messaging ([19852a3](https://github.com/maidsafe/sn_client/commit/19852a343fc287269257a1895b81f659988465cd))

### [0.56.1](https://github.com/maidsafe/sn_client/compare/v0.56.0...v0.56.1) (2021-05-31)

## [0.56.0](https://github.com/maidsafe/sn_client/compare/v0.55.2...v0.56.0) (2021-05-24)


### ⚠ BREAKING CHANGES

* **session:** removing the EndUser registration step from the boostrapping stage

### Features

* **session:** make Sessions agnostic of the keypair used to sign each individual client message ([cbe16fd](https://github.com/maidsafe/sn_client/commit/cbe16fd8ea78bbf6ac44c99831f31ae21629420d))

### [0.55.2](https://github.com/maidsafe/sn_client/compare/v0.55.1...v0.55.2) (2021-05-20)

### [0.55.1](https://github.com/maidsafe/sn_client/compare/v0.55.0...v0.55.1) (2021-05-19)


### Features

* **examples:** add a simple example using Blob API ([5c5e764](https://github.com/maidsafe/sn_client/commit/5c5e764e5052d00301e269d1ff9a27499f23feeb))

## [0.55.0](https://github.com/maidsafe/sn_client/compare/v0.54.13...v0.55.0) (2021-05-19)


### ⚠ BREAKING CHANGES

* **cicd:** This should be bumped with messaging changes

This isn't _actually_ a breaking change, but a bump due to an earlier
commit missing one.

PRs starting with the title `Automated version bump` are auto generated as
part of the CI/CD process and so it is duplicate work running the PR workflow
on them. These changes skip PR CI for them.
This PR also switches the scheduled security audit to only run on the MaidSafe
org repo, not on forks.

### Features

* **cicd:** exclude prs with title 'Automated version bump` ([1e28cf4](https://github.com/maidsafe/sn_client/commit/1e28cf40cbadc394d5ad73f21e91a45add039a60))

### [0.54.13](https://github.com/maidsafe/sn_client/compare/v0.54.12...v0.54.13) (2021-05-19)

### [0.54.12](https://github.com/maidsafe/sn_client/compare/v0.54.11...v0.54.12) (2021-05-18)


### Features

* **errors:** receive CmdErrors from the network ([ee194d5](https://github.com/maidsafe/sn_client/commit/ee194d58f9243e764e581d3f29c067e0bb4722c0))

### [0.54.11](https://github.com/maidsafe/sn_client/compare/v0.54.10...v0.54.11) (2021-05-13)


### Features

* **anti-entropy:** updates for sn_messaging new message enum ([5dfc53c](https://github.com/maidsafe/sn_client/commit/5dfc53cd4618affa63271bab88f59c954b8fcde1))


### Bug Fixes

* rebase atop T5 ([e97ca23](https://github.com/maidsafe/sn_client/commit/e97ca238072fdf69408cdc4181c966f68f863fbe))

### [0.54.10](https://github.com/maidsafe/sn_client/compare/v0.54.9...v0.54.10) (2021-05-12)


### Bug Fixes

* **messaging:** remove all non-bootstrapped elders from local list of elders ([275c353](https://github.com/maidsafe/sn_client/commit/275c353b5fb3595aa7812c3f6ab2066577d63288))

### [0.54.9](https://github.com/maidsafe/sn_client/compare/v0.54.8...v0.54.9) (2021-05-06)


### Features

* **storecost:** handle updated query response ([aa47973](https://github.com/maidsafe/sn_client/commit/aa47973c78f602100567d5946929fa36975ded17))

### [0.54.8](https://github.com/maidsafe/sn_client/compare/v0.54.7...v0.54.8) (2021-05-05)

### [0.54.7](https://github.com/maidsafe/sn_client/compare/v0.54.6...v0.54.7) (2021-05-05)

### [0.54.6](https://github.com/maidsafe/sn_client/compare/v0.54.5...v0.54.6) (2021-05-05)

### [0.54.5](https://github.com/maidsafe/sn_client/compare/v0.54.4...v0.54.5) (2021-05-05)

### [0.54.4](https://github.com/maidsafe/sn_client/compare/v0.54.3...v0.54.4) (2021-05-04)

### [0.54.3](https://github.com/maidsafe/sn_client/compare/v0.54.2...v0.54.3) (2021-05-04)

### [0.54.2](https://github.com/maidsafe/sn_client/compare/v0.54.1...v0.54.2) (2021-05-04)

### [0.54.1](https://github.com/maidsafe/sn_client/compare/v0.54.0...v0.54.1) (2021-05-04)


### Bug Fixes

* **query-listener:** listen to query responses from any of the elders ([b157eee](https://github.com/maidsafe/sn_client/commit/b157eeee20e27db68ccbb0b5ee07c10fc7baf37d))

## [0.54.0](https://github.com/maidsafe/sn_client/compare/v0.53.4...v0.54.0) (2021-05-03)


### ⚠ BREAKING CHANGES

* **deps:** update sn_messaging version to 20.0.1

* **deps:** update sn_messaging version to 20.0.1 ([4c8249d](https://github.com/maidsafe/sn_client/commit/4c8249d22e5cbd2b424dc76bed833656cf39915e))

### [0.53.4](https://github.com/maidsafe/sn_client/compare/v0.53.3...v0.53.4) (2021-04-29)


### Features

* **connMgr:** send queries to the 3 Elders closest to the name of target data ([94526ee](https://github.com/maidsafe/sn_client/commit/94526eede01c3722f671f7b41d43c88dc02cdb75))

### [0.53.3](https://github.com/maidsafe/sn_client/compare/v0.53.2...v0.53.3) (2021-04-29)

### [0.53.2](https://github.com/maidsafe/sn_client/compare/v0.53.1...v0.53.2) (2021-04-28)

### [0.53.1](https://github.com/maidsafe/sn_client/compare/v0.53.0...v0.53.1) (2021-04-28)

## [0.53.0](https://github.com/maidsafe/sn_client/compare/v0.52.19...v0.53.0) (2021-04-28)


### ⚠ BREAKING CHANGES

* **deps:** the updated dependencies have breaking changes

* **deps:** update sn_messaging to 19.0.1 and sn_data_types to ([91709e5](https://github.com/maidsafe/sn_client/commit/91709e555c9747629d3cacc3b1b9e91246b244e7))

### [0.52.19](https://github.com/maidsafe/sn_client/compare/v0.52.18...v0.52.19) (2021-04-27)

### [0.52.18](https://github.com/maidsafe/sn_client/compare/v0.52.17...v0.52.18) (2021-04-27)


### Bug Fixes

* **deps:** use released version of qp2p instead of git branch ([c9b2392](https://github.com/maidsafe/sn_client/commit/c9b23920aa1acb13fc62c488f5d4c1b1fb82bf53))

### [0.52.17](https://github.com/maidsafe/sn_client/compare/v0.52.16...v0.52.17) (2021-04-26)

### [0.52.16](https://github.com/maidsafe/sn_client/compare/v0.52.15...v0.52.16) (2021-04-22)


### Bug Fixes

* **test:** fix assertion in blob_deletions test ([ad7d2ab](https://github.com/maidsafe/sn_client/commit/ad7d2ab7d46fc114856be799f7914ed4d640ce3c))

### [0.52.15](https://github.com/maidsafe/sn_client/compare/v0.52.14...v0.52.15) (2021-04-22)

### [0.52.14](https://github.com/maidsafe/sn_client/compare/v0.52.13...v0.52.14) (2021-04-21)


### Features

* **api:** adding new Register data type API ([c567542](https://github.com/maidsafe/sn_client/commit/c567542a49dc728f2e208152093f454dc4907715))

### [0.52.13](https://github.com/maidsafe/sn_client/compare/v0.52.12...v0.52.13) (2021-04-21)

### [0.52.12](https://github.com/maidsafe/sn_client/compare/v0.52.11...v0.52.12) (2021-04-21)

### [0.52.11](https://github.com/maidsafe/sn_client/compare/v0.52.10...v0.52.11) (2021-04-20)


### Features

* **api:** expose Blob utility API to be able to generate a data map without needing to connect to the network ([817efee](https://github.com/maidsafe/sn_client/commit/817efee20a6d4ff3f1170d0c3142f71891389e79))

### [0.52.10](https://github.com/maidsafe/sn_client/compare/v0.52.9...v0.52.10) (2021-04-12)

### [0.52.9](https://github.com/maidsafe/sn_client/compare/v0.52.8...v0.52.9) (2021-04-08)

### [0.52.8](https://github.com/maidsafe/sn_client/compare/v0.52.7...v0.52.8) (2021-04-08)

### [0.52.7](https://github.com/maidsafe/sn_client/compare/v0.52.6...v0.52.7) (2021-04-07)


### Bug Fixes

* **bootstrap:** reverting previous change to again wrap endpoint.connect_to() with a timeout ([0d51c57](https://github.com/maidsafe/sn_client/commit/0d51c57bba21b5ef914576d537db5ba3ac6fddc7))

### [0.52.6](https://github.com/maidsafe/sn_client/compare/v0.52.5...v0.52.6) (2021-04-07)

### [0.52.5](https://github.com/maidsafe/sn_client/compare/v0.52.4...v0.52.5) (2021-04-07)


### Bug Fixes

* **bootstrap:** wrap endpoint.connect_to() with a timeout ([7ea75b1](https://github.com/maidsafe/sn_client/commit/7ea75b13fcdad521ab49854f964694bc58d85227))

### [0.52.4](https://github.com/maidsafe/sn_client/compare/v0.52.3...v0.52.4) (2021-04-07)


### Bug Fixes

* **connection_manager:** set forward-port to true to use public address ([0e5a21f](https://github.com/maidsafe/sn_client/commit/0e5a21f0642952390982d69d25e6c2781c039c04))

### [0.52.3](https://github.com/maidsafe/sn_client/compare/v0.52.2...v0.52.3) (2021-04-02)


### Bug Fixes

* create listeners before sending queries ([2651bfb](https://github.com/maidsafe/sn_client/commit/2651bfb9715ebf86cce7f683b4fe27eb7dadba1b))

### [0.52.2](https://github.com/maidsafe/sn_client/compare/v0.52.1...v0.52.2) (2021-04-02)


### Bug Fixes

* Logging during conn attempts fixed ([fdeb84f](https://github.com/maidsafe/sn_client/commit/fdeb84f3c125d5774f77d59293f4d1ff64e7e6e3))
* TEMP_HACK: use random port to not throw conn pool out of whack ([311fa30](https://github.com/maidsafe/sn_client/commit/311fa301b5b932b5ed5ec03ff216360742b8624b))

### [0.52.1](https://github.com/maidsafe/sn_client/compare/v0.52.0...v0.52.1) (2021-04-01)

## [0.52.0](https://github.com/maidsafe/sn_client/compare/v0.51.5...v0.52.0) (2021-04-01)


### ⚠ BREAKING CHANGES

* **deps:** the qp2p update includes a breaking change

* **deps:** update qp2p version ([c91c555](https://github.com/maidsafe/sn_client/commit/c91c555a3fe3e4a2faf543134aa1ee322fbde158))

### [0.51.5](https://github.com/maidsafe/sn_client/compare/v0.51.4...v0.51.5) (2021-03-25)


### Bug Fixes

* **bootstrap:** fix bootstrap logic when we get SectionInfo::Redirect ([cd6a24e](https://github.com/maidsafe/sn_client/commit/cd6a24ef46936fde1879bbb6df7b4feeb3ade24d))

### [0.51.4](https://github.com/maidsafe/sn_client/compare/v0.51.3...v0.51.4) (2021-03-24)

### [0.51.3](https://github.com/maidsafe/sn_client/compare/v0.51.2...v0.51.3) (2021-03-24)


### Features

* use known vs all elders, supermajority ([c8ba2b5](https://github.com/maidsafe/sn_client/commit/c8ba2b57d53a0c2b9228223777829b8a9723b61c))
* use supermajority for assesing responses ([8659f62](https://github.com/maidsafe/sn_client/commit/8659f62cea16ddf3ac840c11f6f23cf2e105f916))


### Bug Fixes

* dont connect to elders until we have had a full section response/pk ([a3ec50e](https://github.com/maidsafe/sn_client/commit/a3ec50e1be7110995e65234fa4f7888e9aac712e))

### [0.51.2](https://github.com/maidsafe/sn_client/compare/v0.51.1...v0.51.2) (2021-03-23)

### [0.51.1](https://github.com/maidsafe/sn_client/compare/v0.51.0...v0.51.1) (2021-03-23)


### Bug Fixes

* transfer listener cleanup should happen only once ([66454f7](https://github.com/maidsafe/sn_client/commit/66454f72e675e57d208688068c2f87d00c61fb03))

## [0.51.0](https://github.com/maidsafe/sn_client/compare/v0.50.3...v0.51.0) (2021-03-22)


### ⚠ BREAKING CHANGES

* DT update. Messaging updates

* DT dep update ([ded2602](https://github.com/maidsafe/sn_client/commit/ded260297119a6025b9dcac92889ca3ebf09afc5))

### [0.50.3](https://github.com/maidsafe/sn_client/compare/v0.50.2...v0.50.3) (2021-03-18)


### Bug Fixes

* threshold and error text ([11151d8](https://github.com/maidsafe/sn_client/commit/11151d8b448f3dede5e52cb9dd7b3f674cc348dd))

### [0.50.2](https://github.com/maidsafe/sn_client/compare/v0.50.1...v0.50.2) (2021-03-18)


### Features

* **elders:** remove hard coded elder count ([41b986b](https://github.com/maidsafe/sn_client/commit/41b986ba38ca1b2a2ee3c4f130bad82b22c5d950))

### [0.50.1](https://github.com/maidsafe/sn_client/compare/v0.50.0...v0.50.1) (2021-03-16)


### Bug Fixes

* **bootstrap:** connect to all nodes and dont overwrite qp2p ([bcb31bd](https://github.com/maidsafe/sn_client/commit/bcb31bd410172c9f8c1245a9389b70776f9b7d6a))

## [0.50.0](https://github.com/maidsafe/sn_client/compare/v0.49.0...v0.50.0) (2021-03-11)


### ⚠ BREAKING CHANGES

* **tokio:** new Tokio runtime version is not backward compatible with tokio versions < 1.

* **tokio:** upgrade qp2p to v0.10.1 and tokio to v1.3.0 ([07ce604](https://github.com/maidsafe/sn_client/commit/07ce6045f371b3cdef7c8f23c027b304b506cb2a))

## [0.49.0](https://github.com/maidsafe/sn_client/compare/v0.48.1...v0.49.0) (2021-03-10)


### ⚠ BREAKING CHANGES

* **api:** Policy mutation APIs are removed.

### Features

* **api:** removing APIs that are meant for mutating Seuquence's Policy as they are now immutable ([9ad657b](https://github.com/maidsafe/sn_client/commit/9ad657b366b754c08772c2a446e7e9f7ceff57ea))

### [0.48.1](https://github.com/maidsafe/sn_client/compare/v0.48.0...v0.48.1) (2021-03-04)

## [0.48.0](https://github.com/maidsafe/sn_client/compare/v0.47.2...v0.48.0) (2021-02-25)


### ⚠ BREAKING CHANGES

* **deps:** new version of sn_messaging includes a breaking change

### update

* **deps:** update to the latest version of sn_messaging ([4882ad0](https://github.com/maidsafe/sn_client/commit/4882ad0986c186d7c7d539b2fb1fb9f5fe73dce2))

### [0.47.2](https://github.com/maidsafe/sn_client/compare/v0.47.1...v0.47.2) (2021-02-24)

### [0.47.1](https://github.com/maidsafe/sn_client/compare/v0.47.0...v0.47.1) (2021-02-23)

## [0.47.0](https://github.com/maidsafe/sn_client/compare/v0.46.14...v0.47.0) (2021-02-22)


### ⚠ BREAKING CHANGES

* **deps:** updated sn_messaging version

### Features

* **bootstrap:** update for changes to bootstrap flow ([5af7cbe](https://github.com/maidsafe/sn_client/commit/5af7cbe255722dd7ddcf1a7f7334e317aa7c03d6))
* us our section pk when messaging ([c917b10](https://github.com/maidsafe/sn_client/commit/c917b108733c5765e520f6370ce4f336e8ae7ef2))


### Bug Fixes

* set response elected flag after electing best of the rest ([27726ee](https://github.com/maidsafe/sn_client/commit/27726eeb063500b48116d680659434429771045d))
* **connection_manager:** remove incorrect cloning of session ([67060d1](https://github.com/maidsafe/sn_client/commit/67060d1cb3d67f53d7d3653328ee3acd954deea0))
* clippy ([d060aac](https://github.com/maidsafe/sn_client/commit/d060aac99ab1e4d209e43f5d3f67f2c3b4883aa3))
* double serialization bug ([7ef69a6](https://github.com/maidsafe/sn_client/commit/7ef69a6c224f3a5d25a366f0975acabecf83c662))
* remove unimplemented for match wildcard ([ea22dc8](https://github.com/maidsafe/sn_client/commit/ea22dc80365abee3e164531e18f40d194b30c0b6))
* use msg id new fn for random id ([103beb5](https://github.com/maidsafe/sn_client/commit/103beb584fac08d9e58cd638c54206406eabe72a))


* **deps:** update sn_transfers, sn_messaging, sn_data_types ([4b5d876](https://github.com/maidsafe/sn_client/commit/4b5d876aea68f6252c100f13c6766ea38e67d2d4))

### [0.46.14](https://github.com/maidsafe/sn_client/compare/v0.46.13...v0.46.14) (2021-02-22)

### [0.46.13](https://github.com/maidsafe/sn_client/compare/v0.46.12...v0.46.13) (2021-02-19)

### [0.46.12](https://github.com/maidsafe/sn_client/compare/v0.46.11...v0.46.12) (2021-02-16)

### [0.46.11](https://github.com/maidsafe/sn_client/compare/v0.46.10...v0.46.11) (2021-02-16)

### [0.46.10](https://github.com/maidsafe/sn_client/compare/v0.46.9...v0.46.10) (2021-02-15)

### [0.46.9](https://github.com/maidsafe/sn_client/compare/v0.46.8...v0.46.9) (2021-02-11)


### Features

* **config:** read config file from an optionally provided path ([8d8724b](https://github.com/maidsafe/sn_client/commit/8d8724ba8824d91bc38a16dd144311005698b249))

### [0.46.8](https://github.com/maidsafe/sn_client/compare/v0.46.7...v0.46.8) (2021-02-11)

### [0.46.7](https://github.com/maidsafe/sn_client/compare/v0.46.6...v0.46.7) (2021-02-10)


### Features

* use redirected addesses for elders ([cbd89b5](https://github.com/maidsafe/sn_client/commit/cbd89b564da12d42fdbd62b4af92f80e6bf26cb4))


### Bug Fixes

* doc tests ([e70db31](https://github.com/maidsafe/sn_client/commit/e70db31856d3951a6127ac46e73d8ae754bee112))

### [0.46.6](https://github.com/maidsafe/sn_client/compare/v0.46.5...v0.46.6) (2021-02-09)


### Features

* **test:** enable logger in tests using tracing-subscriber ([448522b](https://github.com/maidsafe/sn_client/commit/448522b7e994df7c13b5203ce7326c40aad900de))

### [0.46.5](https://github.com/maidsafe/sn_client/compare/v0.46.4...v0.46.5) (2021-02-08)

### [0.46.4](https://github.com/maidsafe/sn_client/compare/v0.46.3...v0.46.4) (2021-02-08)

### [0.46.3](https://github.com/maidsafe/sn_client/compare/v0.46.2...v0.46.3) (2021-02-08)


### Features

* remove logging implementation ([cc320a0](https://github.com/maidsafe/sn_client/commit/cc320a04f01625f7a0e94d5c7df32e5d5d990fc8))

### [0.46.2](https://github.com/maidsafe/sn_client/compare/v0.46.1...v0.46.2) (2021-02-04)

### [0.46.1](https://github.com/maidsafe/sn_client/compare/v0.46.0...v0.46.1) (2021-02-03)


### Bug Fixes

* doc tests ([deffab3](https://github.com/maidsafe/sn_client/commit/deffab301c2c18f02c8ce283ffac415cc5fca745))

## [0.46.0](https://github.com/maidsafe/sn_client/compare/v0.45.0...v0.46.0) (2021-02-01)


### ⚠ BREAKING CHANGES

* rename money to token

* rename money to token ([62f816a](https://github.com/maidsafe/sn_client/commit/62f816a5552b09822745c7f50b4d9b9c73824aca))

## [0.45.0](https://github.com/maidsafe/sn_client/compare/v0.44.24...v0.45.0) (2021-02-01)


### ⚠ BREAKING CHANGES

* This updates client creation, Arc<Keypair> is no longer
needed, as they keypair itself contains the Arcs we need.

### Features

* Arc<Keypair>->Keypair updates to accommodate Dt and transfers ([dd23579](https://github.com/maidsafe/sn_client/commit/dd2357943f511a6fd90af837fea208bb1d9a4741))

### [0.44.24](https://github.com/maidsafe/sn_client/compare/v0.44.23...v0.44.24) (2021-01-29)


### Bug Fixes

* adjust for dt updates ([19d77a9](https://github.com/maidsafe/sn_client/commit/19d77a9b35afb9e31a0c42a0bbb0694934fdeac5))
* remove unused method ([ae2453e](https://github.com/maidsafe/sn_client/commit/ae2453e4b91a6972218ac9a393865d7224b9ad5e))

### [0.44.23](https://github.com/maidsafe/sn_client/compare/v0.44.22...v0.44.23) (2021-01-29)


### Features

* set simulated-payouts as a default feature for now ([de6b2c9](https://github.com/maidsafe/sn_client/commit/de6b2c93fc994e0166943199e991befee923df80))

### [0.44.22](https://github.com/maidsafe/sn_client/compare/v0.44.21...v0.44.22) (2021-01-28)


### Bug Fixes

* use latest qp2p & so dont remove good connections from pool ([fa8fbd2](https://github.com/maidsafe/sn_client/commit/fa8fbd2573840533f23186f7b9764150863027d4))

### [0.44.21](https://github.com/maidsafe/sn_client/compare/v0.44.20...v0.44.21) (2021-01-28)

### [0.44.20](https://github.com/maidsafe/sn_client/compare/v0.44.19...v0.44.20) (2021-01-26)


### Features

* update client default config for idle/keep alive time changes. ([547dbdd](https://github.com/maidsafe/sn_client/commit/547dbdd2c7e77b66c8cc5715961c9c68d0fceaf2))
* update elder listeners when incoming messages available ([90f36ee](https://github.com/maidsafe/sn_client/commit/90f36eed6b98b5329f997a22b2c76518a2adc205))

### [0.44.19](https://github.com/maidsafe/sn_client/compare/v0.44.18...v0.44.19) (2021-01-21)


### Features

* warn when we have an unused incoming message stream ([d348a57](https://github.com/maidsafe/sn_client/commit/d348a57729cabbd4e8ac366a901e7d0cdefee45e))

### [0.44.18](https://github.com/maidsafe/sn_client/compare/v0.44.17...v0.44.18) (2021-01-21)

### [0.44.17](https://github.com/maidsafe/sn_client/compare/v0.44.16...v0.44.17) (2021-01-18)


### Features

* **error:** re-export sn_messaging::Error as ErrorMessage on the public API ([e3829b0](https://github.com/maidsafe/sn_client/commit/e3829b0d5d00cc262ca69fef92a1670118162a52))

### [0.44.16](https://github.com/maidsafe/sn_client/compare/v0.44.15...v0.44.16) (2021-01-18)

### [0.44.15](https://github.com/maidsafe/sn_client/compare/v0.44.14...v0.44.15) (2021-01-18)

### [0.44.14](https://github.com/maidsafe/sn_client/compare/v0.44.13...v0.44.14) (2021-01-18)


### Features

* listen for bootstrap response on IncomingMessages also ([f880f98](https://github.com/maidsafe/sn_client/commit/f880f9823e77b3727253f9dee01a304cc4e3eddd))
* **connections:** updates to listen to all messages from each elder ([76c1836](https://github.com/maidsafe/sn_client/commit/76c1836db1eda7cc98e99bdef3d3c336fa03ab7f))


### Bug Fixes

* responses require threshold again ([d86bce4](https://github.com/maidsafe/sn_client/commit/d86bce44b2f59a76942b50d50439f2322a450641))
* **responses:** Remove unneeded shortcircuit and clarify logs in repsonse handling ([a86bbcd](https://github.com/maidsafe/sn_client/commit/a86bbcda6517a25b2080696b0890cf826d86fd65))

### [0.44.13](https://github.com/maidsafe/sn_client/compare/v0.44.12...v0.44.13) (2021-01-05)

### [0.44.12](https://github.com/maidsafe/sn_client/compare/v0.44.11...v0.44.12) (2021-01-05)


### Features

* **errors:** Use updated sn_messaging ([e513ab3](https://github.com/maidsafe/sn_client/commit/e513ab3d737ac43b024d3216a689f36f3af476c6))

### [0.44.11](https://github.com/maidsafe/sn_client/compare/v0.44.10...v0.44.11) (2020-12-30)

### [0.44.10](https://github.com/maidsafe/sn_client/compare/v0.44.9...v0.44.10) (2020-12-29)


### Features

* **errors:** use thiserror for error creation ([bc093b6](https://github.com/maidsafe/sn_client/commit/bc093b6fc5cb43fe1bdfa8214e2f1907935e98a0))

### [0.44.9](https://github.com/maidsafe/sn_client/compare/v0.44.8...v0.44.9) (2020-12-28)

### [0.44.8](https://github.com/maidsafe/sn_client/compare/v0.44.7...v0.44.8) (2020-12-28)


### Features

* **blob:** remove local blob cache ([8a1b871](https://github.com/maidsafe/sn_client/commit/8a1b871ebf70ce5ebcf8aaa9146705b29927f925))

### [0.44.7](https://github.com/maidsafe/sn_client/compare/v0.44.6...v0.44.7) (2020-12-24)

### [0.44.6](https://github.com/maidsafe/sn_client/compare/v0.44.5...v0.44.6) (2020-12-17)


### Bug Fixes

* choose longest history w/ simu payout responses ([d22aa01](https://github.com/maidsafe/sn_client/commit/d22aa0153518d9588b229234f634042baa4c22b4))
* CmdError handling for validation reqs ([a7a7ad4](https://github.com/maidsafe/sn_client/commit/a7a7ad4e732b2d6e8b6fa24397ada97c4593eb8d))

### [0.44.5](https://github.com/maidsafe/sn_client/compare/v0.44.4...v0.44.5) (2020-12-09)

### [0.44.4](https://github.com/maidsafe/sn_client/compare/v0.44.3...v0.44.4) (2020-12-09)

### [0.44.3](https://github.com/maidsafe/sn_client/compare/v0.44.2...v0.44.3) (2020-12-07)

### [0.44.2](https://github.com/maidsafe/sn_client/compare/v0.44.1...v0.44.2) (2020-12-07)

### [0.44.1](https://github.com/maidsafe/sn_client/compare/v0.44.0...v0.44.1) (2020-12-07)

### [0.44.0](https://github.com/maidsafe/sn_client/compare/v0.43.0...v0.44.0) (2020-12-07)


### Features

* add configurable bootstrap to client ([5ad120b](https://github.com/maidsafe/sn_client/commit/5ad120bfc7e734b543fafcb96acf877b32adaeb4))
* remove seq cache ([afc516b](https://github.com/maidsafe/sn_client/commit/afc516b6cb2e8ec0c54a9dc2232f21818ad802b8))
* update lseq data type ([b064eff](https://github.com/maidsafe/sn_client/commit/b064eff303f43c3f1f98d22c1b43aee8dba64b5c))
* **api:** Add get_balance_for api for specificly PK requests ([78847f8](https://github.com/maidsafe/sn_client/commit/78847f8c3e289a87b9088be9f2d166ede11bfad1))
* **apis:** remove get_seq/unseq_map apis, and go for cleaner get_map ([3b47500](https://github.com/maidsafe/sn_client/commit/3b4750082e9ea21193f098045ebac31a27d1dc03))
* **arc:** Require an arc wrapped keypair for init ([38e7ef3](https://github.com/maidsafe/sn_client/commit/38e7ef32ac416336af853cf663a82d57b919c8c3))
* **blob:** expose self_ecnrypt API for dry run ([d3abe53](https://github.com/maidsafe/sn_client/commit/d3abe53d28ee15c1cb758399153e6c6a91a52165))
* **ci:** auto generate dependency graph via CI ([ac13840](https://github.com/maidsafe/sn_client/commit/ac13840c0bcee2db67c38275b83eef2be3e3f24f))
* **conn:** make query response threhsold dynamic ([ebf310a](https://github.com/maidsafe/sn_client/commit/ebf310a38b9506f7241a4c7d4296ee0d14ed28f5))
* **connection_manager:** improve handling of connections ([158ba06](https://github.com/maidsafe/sn_client/commit/158ba0690451e34ed5bdb10e7c771602b1b501fb))
* **connections:** set up listener for events/errors ([deeecc6](https://github.com/maidsafe/sn_client/commit/deeecc62bb65e99663683f6b2712c1156420adbc))
* **err_listener:** implement CmdError listener and fix map data tests ([b57ba9a](https://github.com/maidsafe/sn_client/commit/b57ba9ad2780b280dc884e609b423a091fc8296b))
* **errors:** add error for insufficient elder connections ([357ca33](https://github.com/maidsafe/sn_client/commit/357ca33290f3ab19edfbb3d08f6414004b5a142f))
* **listen:** Initial implementation of listen on network ([b38c9bf](https://github.com/maidsafe/sn_client/commit/b38c9bf922f0a10480e13c98076c6a8b2fa70f18))
* **map:** refactoring Map API ([6b8cabc](https://github.com/maidsafe/sn_client/commit/6b8cabc5c51e7ead597035ede8e4e9676bed8b46))
* **qp2p:** Inital set up to enable listeners for qp2p ([63adbc7](https://github.com/maidsafe/sn_client/commit/63adbc7cbca5736850c880cb2316202bffebd94a))
* **qp2p:** update qp2p version ([41958b3](https://github.com/maidsafe/sn_client/commit/41958b3a0bbcbcc6be9b3ff853d858ae476680d1))
* **rand:** use OsRng instead of thread ([437340a](https://github.com/maidsafe/sn_client/commit/437340af6736d47b1650f6054a3930c60acc298b))
* **self-encrypt:** re add self encryption to client blob apis ([e550dad](https://github.com/maidsafe/sn_client/commit/e550dad3137d240d901077f04bc8cde1a23eed3c))
* **seq:** Sign ops before applying locally + sending to network ([08d43c8](https://github.com/maidsafe/sn_client/commit/08d43c8a35643f25aecd5dc9c03911d1d2291067))
* **seq:** Update to sn_data_types and update seq apis ([ad248a7](https://github.com/maidsafe/sn_client/commit/ad248a7e7fa6ab015ca02f61075642e6dc2ee619))
* **seq:** Use signed ops for sequence append ([62c7d46](https://github.com/maidsafe/sn_client/commit/62c7d46fbd1b11aafac495a26ccabf8dbc6da1df))
* **transfer_id:** Provide u64 and pk of transfer to be used as id ([7bcd6b3](https://github.com/maidsafe/sn_client/commit/7bcd6b310b8fad52124b537a88fc74222b2f66de))
* **transfers:** impl DebitAgreementProof aggregator ([8ad8c39](https://github.com/maidsafe/sn_client/commit/8ad8c395f8ac9838cbba3a71c08b86644cbce647))
* **transfers:** impl StoreCost for data writes ([efaf2b0](https://github.com/maidsafe/sn_client/commit/efaf2b03b2dae6b02ffbc428fb2d816adf3bc8ae))
* instantiate the client w/ fullId not just sk ([79f064f](https://github.com/maidsafe/sn_client/commit/79f064f75e6b106ef3bc04357041b963303f0f9e))


### Bug Fixes

* **blob:** fix blob flows and simplify the self_encryption process for ([423002e](https://github.com/maidsafe/sn_client/commit/423002edb99691f8b32b91d4441b1869644d92ff))
* **blob:** fix bug in blog storage ([d7dec70](https://github.com/maidsafe/sn_client/commit/d7dec705f63221faff2f538263eb87962291885f))
* **blob:** fix writing blob with always our PublicKey ([ae2aea4](https://github.com/maidsafe/sn_client/commit/ae2aea4150d7f33959e08dfb29626ec42ef2ed5c))
* **ci:** fix dependency graph script ([0e178e2](https://github.com/maidsafe/sn_client/commit/0e178e267a726a9d293dcd8b6a8e777dc9a3e8db))
* **connections:** Dont fail when one elder cannot connect ([cf4db2c](https://github.com/maidsafe/sn_client/commit/cf4db2c465aade7ab45443758bd2ae0ebc2a5ed9))
* **connections:** retry failed connections ([f14ebe6](https://github.com/maidsafe/sn_client/commit/f14ebe6b6bee0e035bb0e485753cc8810ce54c53))
* **data_types:** update to new MsgEnvelope ([4d53f14](https://github.com/maidsafe/sn_client/commit/4d53f147c026015fe5c6092695edf75f38b46378))
* **msgs:** fix random msg id generation ([624a5b0](https://github.com/maidsafe/sn_client/commit/624a5b058d4930f9e417ef33e603373e715d7378))
* **new:** Fix simulated payout dot and related tests ([a795d4a](https://github.com/maidsafe/sn_client/commit/a795d4a02fd6c2258534f5b635b8d88a7793f2b9))
* **qp2p:** Fixes for latest qp2p master ([0a5001b](https://github.com/maidsafe/sn_client/commit/0a5001b2fa21e22513a37621e7f35636fe6d840e))
* **seq:** sign op.crdt_op ([a584ef0](https://github.com/maidsafe/sn_client/commit/a584ef0c3ed672cce2cfc5bfee980c681d00d0f6))
* **test:** don't hide exported tests behind #[cfg(test)] ([40d0766](https://github.com/maidsafe/sn_client/commit/40d0766efa2e57013c117e565c01f149dc455d4d))
* **tests:** fix actor and Money transfer tests ([ad67e08](https://github.com/maidsafe/sn_client/commit/ad67e08ebdb981c9558d6b37c39503641defdbd0))
* add testing feature flag again ([21412e0](https://github.com/maidsafe/sn_client/commit/21412e09563daca70585d731ef8cdec9d941ab01))

### [0.43.0](https://github.com/maidsafe/sn_client/compare/safe_core-0.42.1-safe_auth-0.17.1-safe_app-0.17.1...v0.43.0) (2020-07-30)
* fix/clippy: fix minor clippy fix

### [0.42.1] (2020-07-16)
* Update ffi-utils to 0.17.0

### [0.42.0]
* Added of SequenceData APIs
* Removed of AppendOnlyData APIs
* Standardize cargo dependency versioning

### [0.41.3]
* Fix CI deploy

### [0.41.2]
* Update the number of responses required to process a request.

### [0.41.1]
* Update quic-p2p to 0.6.2
* Update sn_data_types to 0.9.0
* Refactor to use updated request/response types

### [0.41.0]
* Use Async/await rust.

### [0.40.0]
* Update quic-p2p to 0.5.0
* Attempt to bootstrap multiple times before returning an error

### [0.39.0]
* Add position and index to get_value
* Refactor the connection manager to use new quic-p2p API
* Always use random port instead of default
* Implement multi-vault connection manager
* Implement the new handshake protocol and manage connection state transitions
* Remove unused imports and linting
* Remove macro_use style
* Add support for GET_NEXT_VERSION in more places
* Expose a new `gen_data_map` API which generates a file's data map without putting the chunks on the network
* Make returned error codes to be positive numbers
* Remove pedantic warnings

### [0.38.1]
* Fix broken master workflow

### [0.38.0]
* Update to sn_data_types 0.7.2
* Update to lazy_static 1.4.0
* Update ffi_utils to 0.15.0
* Use GHA for Android libs build
* Expose `gen_data_map` API which generates a file's data map without putting the chunks on the network

### [0.37.3]
* Make another fix to automatic publishing

### [0.37.2]
* Refactor and reenable client mock tests
* Fix automatic publishing

### [0.37.1]
* Fix automatic deploys and releases

### [0.37.0]
* Remove Rust Sodium dependency

### [0.36.0]
* Update to quic-p2p 0.3.0
* Add `set_config_dir_path` API to set a custom path for configuration files.
* Deprecate the `maidsafe_utilities` and `config_file_handler` dependencies.
* Migrate to GitHub actions for CI / CD for all platforms except Mac OS builds.
* Fix inconsistency with real vault.

### [0.35.0]
* Remove unused `routing` module and fix errors
* Rework MDataKey and MDataValue to use FFI conventions
* Make miscellaneous doc fixes
* Clean up FFI documentation

### [0.34.0]
* Technical release to solve some issues in our automated publishing process

### [0.33.0]
* Remove Routing dependency from safe_core.
* Use quic-p2p for communication with Vaults.
* Use new data types from sn_data_types (AppendOnlyData and unpublished ImmutableData).
* Add Safecoin-related tests and features.
* Use the `stable` branch of the Rust compiler and Rust edition 2018.

### [0.32.1]
* Move module-level documentation to wiki, replace with link.
* Make general documentation fixes and improvements.
* Fix some compiler errors.

### [0.32.0]
* Switch to base32 encodings for case-insensitive URIs for IPC
* Send a mock bit with ipc messages so that mock and non-mock components trying to communicate results in an error
* Fix the mock-routing bug which was resulting in corrupted MockVault files
* Remove `is_mock_build` function, replace with `auth_is_mock` and `app_is_mock`

### [0.31.0]
* Refactor `Client` struct to a trait for a better separation of concerns
* Implement `CoreClient` as a bare-bones network client for tests
* Move Authenticator-related `Client` functions to `safe_authenticator`

### [0.30.0]
* Use rust 1.26.1 stable / 2018-02-29 nightly
* rustfmt-nightly 0.8.2 and clippy-0.0.206
* Updated license from dual Maidsafe/GPLv3 to GPLv3
* Add `MDataEntry` struct
* Implement bindings generation

### [0.29.0]
* Use rust 1.22.1 stable / 2018-01-10 nightly
* rustfmt 0.9.0 and clippy-0.0.179
* Fix naming conventions in callback parameters and elsewhere

### [0.28.0]
* Move `AccessContainerEntry` to safe_core
* Add FFI wrapper for `MDataInfo`
* Add access container entry to `AuthGranted`
* Add `MDataKey` and `MDataValue` structs
* Add function for checking mock-routing status of build
* Add config file functionality with options for unlimited mock mutations, in-memory mock storage, and custom mock vault path.
* Add environment variables to override config options for unlimited mock mutations and custom mock vault path.
* Add support for providing arbitrary user data along with `IpcReq::Unregistered` auth request
* Improve documentation for callback parameters
* Improve NFS tests
* Remove unnecessary constants equivalent to environment variables names

### [0.27.0]
* Improve documentation and fix bugs
* Nonce in the MDataInfo struct is no longer optional. This is a breaking external change
* Remove of the neccessity to pass `--feature testing` to run tests
* Replace all secret keys with drop-in equivalents that implement secure cloning. They don't actually clone the underlying data but instead implicitly share it.

### [0.26.2]
* Update routing to 0.33.2

### [0.26.1]
* Update routing to 0.33.1
* Fix mock vault write mode

### [0.26.0]
* Update routing to 0.33.0
* Decryption in MDataInfo tries both the new and old encryption keys before giving up
* Env var to control in-mem or on-disk storage for mock vault
* Change and improve account packet structure
* Fix mock vault deserialisation

### [0.25.1]
* Update routing to 0.32.2

### [0.25.0]
* Add new functions for operations recovery in the `safe_core::recovery` module (e.g. if a `mutate_mdata_entries` operation is failed with the `InvalidSuccessor` error, it will be retried with an increased version)
* Add new testing features to mock-routing (allowing to override certain requests with predefined responses)
* Improve the NFS test coverage
* Update to use Rust Stable 1.19.0 / Nightly 2017-07-20, clippy 0.0.144, and rustfmt 0.9.0
* Update `routing` to 0.32.0 to include more descriptive Map errors
* Update other dependencies

### [0.24.0]
* Use asynchronous I/O and futures for interfacing with Routing
* Deprecate and remove StructuredData and AppendableData types
* Introduce a new data type instead: Map
* Implement URI-based IPC interaction between apps required for supporting mobile devices
* Integrate with routing 0.31.0
* Move all FFI functions to their own separate crates
* Refactor and simplify the NFS module

### [0.23.0]
* Integrate with routing 0.28.5
* Invitation based account creation support in client (API change)
* Invitation-generator and populator example binary
* New error types for ivitation errors
* Serde instead of rustc-serialize in production
* Use chrono instead of time crate (default serde serialisable)
* Fix bugs concerning to unclaimable SD and re-claiming SD via PUT; test cases updated

### [0.22.4]
* Integrate with routing 0.28.4 (0.28.3 is skipped and is yanked from crates).
* Use rust 1.16.0, nightly-2017-03-16 and clippy 0.0.120
* Add a few trace messages for better diagnostics.
* Cleanup README.md

### [0.22.3]
* Integrate with routing 0.28.2

### [0.22.2]
* Integrate with routing 0.28.0

### [0.22.1]
* API to get MAID-Public signing key.

### [0.22.0]
* New error type - MutationError::DataTooLarge.
* New Delete handling and update of code and test cases.
* New APIs - Put to re-claim deleted data (specify version), make data unclaimable.
* Changes and fixes in mock-routing to conform to routing and vaults for error deduction and error types.

### [0.21.2]
* Serialisation and deserialisation for Sign Keys.
* API for getting Filtered keys from AppendableData.
* Fix accidental name mangling of C function.

### [0.21.1]
* Reverting the commit to remove dir-tag from dir-key: commit e829423 reverts commit 4fbc044.
* Trim credentials in examples to not include a `\n`.

### [0.21.0]
* Removal of base64 indirection as we no longer have JSON interface to `safe_core`.
* Many more test cases to thoroughly check low-level-api
* Add new api's wanted by launcher - ownership assertion, version exposure, more serialisations etc.
* Make tag-types for versioned and unversioned StructuredData MaidSafe constants and remove them from `DirectoryKey`.

### [0.20.0]
* API changed from JSON to direct FFI calls for interfacing with other languages.
* Provide low-level-api for finer grained control for manipulation of MaidSafe data types.
* Provide Private & Public Appendable Data operations and manipulations.
* Code APPEND API.
* Update mock-routing to comply with above changes to mimic basic routing and vault functionality for purposes of independent testing.
* Introduce Object Caching - a method in which `safe_core` keeps cache of object in LRU cache and gives only a POD (u64) handle via FFI.
* Increase test cases performace when using mock routing by not writing data to file for test-cases.
* Dependency update - routing updated to 0.26.0.

### [0.19.0]
* Dependency update - routing updated to 0.23.4.
* Log path exposed to FFI so that frontend is intimated where it is expected to create its log files.
* Dependency on rust_sodium instead of sodiumoxide and removal of libsodium instruction from CI builds.

### [0.18.1]
* Dependency update - routing reduced to 0.23.3 and safe_network_common increased to 0.7.0.

### [0.18.0]
* Requests made to safe_core will now timeout after 2 min if there is no response from routing.
* Self_encrypt write used by safe_core via sequential encryptor will now try to put data onto the Network immediately if possible leading to better progress indication across FFI.
* Logging added to safe_core.
* Accessing DNS will not do a bunch of checks which it used to previously because it lead to erroneous corner cases in which one user could not access websites created by other before they created their own DNS first etc.

### [0.17.0]
* Instead of requiring all 3 of PIN, Keyword and Password, have user type only one secure pass-phrase and derive the required credentials internally.

### [0.16.2]
* Expose get-account-info functionality in FFI for launcher to consume.
* Fix sodiumoxide to v0.0.10 as the new released v0.0.12 does not support rustc-serializable types anymore and breaks builds.
* Update dependencies

### [0.16.1]
* Update Routing to 0.23.2
* Add logging to network events.
* Delete existing log file due to issue in v3 of log4rs which instead of truncating/appending overwrites the existing log file garbling it.
* Rustfmt and clippy errors addressed.
* Error recovery test case.
* Extract sub-errors out of Self Encryption errors and convert them to C error codes for FFI.

### [0.16.0]
* Update dependencies
* Refactor FFI as `Box::into_raw()` is stable
* Refactor FFI to deal with pointer to concrete types instead of ptr to void for more type safety
* Fix undefined behaviour in transmute to unrelated type in FFI
* Fix non-termination of background thread which got exposed after fixing the above
* Reorder Imports
* Resolve many Clippy errors
* Expose functionality to collect stats on GETs/PUTs/POSTs/DELETEs
* Error recovery for failure in intermediary steps of a composite operation (like DNS register and delete).

### [0.15.1]
* Upgrade routing to 0.22.0
* Upgrade safe_network_common to 0.3.0

### [0.15.0]
* Upgrade to new routing and self_encryption.

### [0.14.6]
* Merge safe_ffi into safe_core.

### [0.14.5]
* Updating routing to 0.19.1

### [0.14.4]
* Dependency update

### [0.14.3]
* Dependency update

### [0.14.2]
* Pointing and conforming to Routing 0.15.0
* Removal of feature use-mock-crust
* internal code improvement - removing now-a-one-liner function

### [0.14.1]
* Updated dependencies.

### [0.14.0]
* Migrate to Routing 0.13.0.

### [0.13.1]
* Updated dependencies.

### [0.13.0]
* Added minimal support for mock crust.
* Updated dependencies.

### [0.12.1]
* Updated dependencies.

### [0.12.0]
* Integrated with safe_network_common.
* Response handling in case of errors made complete with reason for errors coded in.
* Mock routing updated to give correct reason in cases for errors. All corresponding test cases update to thoroughly test most of scenarios.

### [0.11.0]
* Reintegrated messaging API.
* Fixed a bug in file metadata serialisation which caused the frontend app to crash on Windows.

### [0.10.0]
* Code made more resilient to precision of time resolution on host machines by including dedicated version counter in file metadata. This is also part of public API.
* self_authentication example gives better error message on trying to hijack pre-existing user network name.
* Updated dependencies.

### [0.9.0]
* Updated response handling in line with network behaviour changes.
* Updated dependencies.

### [0.8.0]
* Nfs and Dns modules and examples merged into safe_core.

### [0.7.0]
* Disconnect event detection and translation to ffi compatible value

### [0.6.1]
* self_encryption updated to 0.2.6

### [0.6.0]
* Migrated to Routing 0.7.0
* Switched LOGIN_PACKET_TYPE_TAG to 0

### [0.5.0]
* Refactored to comply with new routing API
* Compiles and passes tests with Mock with stable Rust

### [0.4.0]
* Refactored to comply with new routing API

### [0.3.1]
* Remove wildcard dependencies

### [0.3.0]
* [MAID-1423](https://maidsafe.atlassian.net/browse/MAID-1423) Rename safe_client to safe_core

### [0.2.1]
* Routing crate updated to version 0.4.*

### [0.2.0]
* [MAID-1295](https://maidsafe.atlassian.net/browse/MAID-1295) Remove all unwraps() AND Check for Ok(r#try!( and see if really required (ie., for error conversion etc)
* [MAID-1296](https://maidsafe.atlassian.net/browse/MAID-1296) Remove unwanted errors and Unexpected should take an &str instead of String
* [MAID-1297](https://maidsafe.atlassian.net/browse/MAID-1297) Evaluate test_utils in client
* [MAID-1298](https://maidsafe.atlassian.net/browse/MAID-1298) Put debug statements
* [MAID-1299](https://maidsafe.atlassian.net/browse/MAID-1299) check for all muts (eg., response_getter etc) and validate if really required
* [MAID-1300](https://maidsafe.atlassian.net/browse/MAID-1300) Error conditions in Mock Routing
* [MAID-1301](https://maidsafe.atlassian.net/browse/MAID-1301) Test cases for Error conditions in Mock
* [MAID-1303](https://maidsafe.atlassian.net/browse/MAID-1303) Address the TODO’s and make temporary fixes as permanent (eg., listening to bootstrapped signal)
* [MAID-1304](https://maidsafe.atlassian.net/browse/MAID-1304) Test cases for TODO's and temp fixes as permanent

### [0.1.5]
* Wait for routing to fire a bootstrap completion event
* Added support for environment logger

### [0.1.4]
* [MAID-1219](https://maidsafe.atlassian.net/browse/MAID-1219) Implement Private and Public types
* [MAID-1249](https://maidsafe.atlassian.net/browse/MAID-1249) Implement Unified Structured Datatype
    - [MAID-1252](https://maidsafe.atlassian.net/browse/MAID-1252) Mock Unified StructuredData and ImmutableData
    - [MAID-1253](https://maidsafe.atlassian.net/browse/MAID-1253) Update Mock Routing to support Mock Unified SturcturedData and ImmutableData
    - [MAID-1222](https://maidsafe.atlassian.net/browse/MAID-1222) Compute size of Structured Data
    - [MAID-1223](https://maidsafe.atlassian.net/browse/MAID-1223) Implement a handler for Storing UnVersioned Structured Data
    - [MAID-1224](https://maidsafe.atlassian.net/browse/MAID-1224) Implement a handler for Retrieving Content of UnVersioned Structured Data
    - [MAID-1225](https://maidsafe.atlassian.net/browse/MAID-1225) Write Test Cases for UnVersioned Structured Data handler
    - [MAID-1230](https://maidsafe.atlassian.net/browse/MAID-1230) Implement a handler for Storing Versioned Structured Data
    - [MAID-1231](https://maidsafe.atlassian.net/browse/MAID-1231) Create MaidSafe Specific configuration directory
    - [MAID-1232](https://maidsafe.atlassian.net/browse/MAID-1232) Write Test Cases for Versioned Structured Data handler
    - [MAID-1226](https://maidsafe.atlassian.net/browse/MAID-1226) Implement Session Packet as UnVersioned Structure DataType
    - [MAID-1227](https://maidsafe.atlassian.net/browse/MAID-1227) Update the test cases in Core API
    - [MAID-1228](https://maidsafe.atlassian.net/browse/MAID-1228) Update the test cases in mock routing framework
    - [MAID-1234](https://maidsafe.atlassian.net/browse/MAID-1234) Update Hybrid Encrypt and Decrypt

### [0.1.3]
* [MAID-1283](https://maidsafe.atlassian.net/browse/MAID-1283) Rename repositories from "maidsafe_" to "safe_"

### [0.1.2]
* [MAID-1209](https://maidsafe.atlassian.net/browse/MAID-1209) Remove NFS API

### [0.1.1]
* Updated dependencies' versions
* Fixed lint warnings caused by latest Rust nightly

### [0.1.0] RUST-2 sprint
* Account Creation
    - Register
    - Login
* Implement Storage API
    - Implement types
        - Implement MetaData, File and DirectoryListing types
    - Implement Helpers
        - Directory Helper
            - Save DirectoryListing
            - Get Directory
            - Get Directory Versions
        - File Helper
            - Create File, update file and Metatdata
            - Get Versions
            - Read File
        - Unit test cases for Directory and File Helpers
    - Implement REST DataTypes
        - Container & Blob types
            - Implement Blob and Container types
        - REST API methods in Container
            - Create Container & Get Container
            - List Containers, Update / Get Container Metadata
            - Delete Container
            - Create Blob
            - List Blobs
            - Get Blob
            - Update Blob Content
            - Get Blob Content
            - List Blob Version
            - Delete Blob
            - Copy Blob
            - Update / Get Blob Metadata
        - Unit test cases for API
    - Implement Version Cache (cache key,(blob/container) info to reduce network traffic)
    - Root Directory handling
* Create Example:
    - Self authentication Example
    - Example to demonstrate Storage API
