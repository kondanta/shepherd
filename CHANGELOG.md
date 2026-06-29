# Changelog

## [2026.6.1] - 2026-06-29

### Bug Fixes

- git cliff (#77)([`c6c991e`](https://github.com/kondanta/shepherd/commit/c6c991e1e7836c0c73b38849eaaccbfd0523773b))

- use git-cliff binary (#76)([`f5c4cdc`](https://github.com/kondanta/shepherd/commit/f5c4cdc5c4a88306292b326a123e3a7b8a90ea82))

- generate body for release PR (#75)([`22bc69d`](https://github.com/kondanta/shepherd/commit/22bc69df8022b72031d2991ef8526b2f0fb5046e))

- log successful service update completion in update_service([`8d23580`](https://github.com/kondanta/shepherd/commit/8d23580928431c853d504435bc8aace571558664))

- run fmt([`4f83806`](https://github.com/kondanta/shepherd/commit/4f83806f37905be5edc2e7f89481d74683b18e75))

- validate GitHub API inputs to prevent URL component injection (CWE-99)([`11cdca1`](https://github.com/kondanta/shepherd/commit/11cdca1b51d30d5e48a31eea692a42f0f5ec0125))

- use &Path instead of &PathBuf in write_atomically signature([`8bbfe8f`](https://github.com/kondanta/shepherd/commit/8bbfe8f38190c12a132b699d6c621264bec56f01))

- run blocking filesystem I/O on spawn_blocking threads([`5f95918`](https://github.com/kondanta/shepherd/commit/5f959181f3ec803665dcbf648e686c109596fecf))

- lint issue([`32bfaea`](https://github.com/kondanta/shepherd/commit/32bfaea98062074288d9591965d83bf1d7396b32))

- typo([`e406b62`](https://github.com/kondanta/shepherd/commit/e406b62c17aa603fe6a92a3ed1a97b19b1cd2a2d))


### Code Refactoring

- preserve compose file formatting, dedup auth header, extract write_atomically([`87959d4`](https://github.com/kondanta/shepherd/commit/87959d481c5cc871f296f83fd4ba326dc9e8fa78))

- instrument docker commands ([`01271bb`](https://github.com/kondanta/shepherd/commit/01271bb03c61621adecb5e6cfdc7890d7ff4f35a))

- **tracing:** refactor tracing after config([`d440749`](https://github.com/kondanta/shepherd/commit/d440749f52a0f1bebd32d342134099da7589021f))


### Documentation

- add readme([`72b4326`](https://github.com/kondanta/shepherd/commit/72b4326e98549c9404860f8afd4a31201b2b2c21))


### Features

- automize release cycle (#73)([`0139ed7`](https://github.com/kondanta/shepherd/commit/0139ed74ed71e916aac4e51f5b00a410026e8ac1))

- use axum otel  (#63)([`9abb17d`](https://github.com/kondanta/shepherd/commit/9abb17ddd0cab76e8f6d48642e8dc60895af3e5b))

- use hashset for dedup([`88078de`](https://github.com/kondanta/shepherd/commit/88078de3b1aa03188676c1fa5ef7ee9b1bbcc681))

- compile for arm7([`f463738`](https://github.com/kondanta/shepherd/commit/f463738d75dd88606e866fe1b81a53bbe5cdb02a))

- add service filter (#25)([`e4d51ae`](https://github.com/kondanta/shepherd/commit/e4d51aeebf357d2ddf04714dd7eef7aae99022a8))

- enable custom image for manual deployments([`cbd1854`](https://github.com/kondanta/shepherd/commit/cbd1854cee4760ccea715c5c2fc7b668f0718b39))

- add repo path prefix([`4caa677`](https://github.com/kondanta/shepherd/commit/4caa67714a0ed5d4464174a152093630ae4421ce))

- add polling option([`3989a25`](https://github.com/kondanta/shepherd/commit/3989a25da4801b12fff17b672fcd992322d5005b))

- add docker stuff([`dcfd4e8`](https://github.com/kondanta/shepherd/commit/dcfd4e83a1554bd968ca21e9ae9cea6e4bd18f21))

- add container related commands([`46fb0ab`](https://github.com/kondanta/shepherd/commit/46fb0ab8ca0404326501aadfc4200e455ed990ff)) ⚠ **BREAKING**

- add ci workflow for PRs([`33cd047`](https://github.com/kondanta/shepherd/commit/33cd0477950493f47ca4249eff8b54965e643b4e))

- add justfile([`af34f22`](https://github.com/kondanta/shepherd/commit/af34f22df37263b082cfab74c69c3977ed2e6cb6))

- setup renovate([`7e66f87`](https://github.com/kondanta/shepherd/commit/7e66f879d3edd8e711e109c7856cd143d2136ab2))

- setup server and tracing([`99562ae`](https://github.com/kondanta/shepherd/commit/99562ae92309eb3cd8dc8c78e7f9200a66743a11)) ⚠ **BREAKING**


### Miscellaneous Chores

- **deps:** lock file maintenance (#79)([`fca12da`](https://github.com/kondanta/shepherd/commit/fca12da52645f3c9f62ee95f2040e37e457fc142))

- **deps:** update rust crate arc-swap to v1.9.2 (#78)([`49b5def`](https://github.com/kondanta/shepherd/commit/49b5def17cc57d574a84707ce8b7f9ef4af1edf2))

- **deps:** update docker:cli docker digest to 862099a (#72)([`82184dd`](https://github.com/kondanta/shepherd/commit/82184dd9651a7159161bad17163c6cef09910ac2))

- update dependencies (#71)([`6c65cf3`](https://github.com/kondanta/shepherd/commit/6c65cf314dd58d0751b068cca883c7b9b2557171))

- **deps:** update rust crate opentelemetry-semantic-conventions to v0.32.1 (#70)([`a2be0ef`](https://github.com/kondanta/shepherd/commit/a2be0ef1ed00ef21ba0dfd37f5d673627c86b5b9))

- **deps:** lock file maintenance (#65)([`afd4e2e`](https://github.com/kondanta/shepherd/commit/afd4e2e7685385256dc61bbccfecddd156221f14))

- **dependency:** update dependencies (#69)([`6e745c2`](https://github.com/kondanta/shepherd/commit/6e745c2040f6974c4ec0e5e371075f324040b526))

- **deps:** update lukemathwalker/cargo-chef:latest-rust-alpine docker digest to 7447ff7 (#68)([`8d71d60`](https://github.com/kondanta/shepherd/commit/8d71d6067be2cc8373ff2d0b52016d01487850d0))

- **deps:** update docker:cli docker digest to d14410a (#67)([`9933709`](https://github.com/kondanta/shepherd/commit/9933709d50ea80619f6789c3b0b54e414c9df780))

- **deps:** update docker:cli docker digest to 873de13 (#64)([`6038405`](https://github.com/kondanta/shepherd/commit/6038405906a554f86115964b8817731e3e2f0364))

- **deps:** lock file maintenance (#55)([`f0d55af`](https://github.com/kondanta/shepherd/commit/f0d55af767af8d7a0253a91a04eb6a6180d2e806))

- **deps:** update lukemathwalker/cargo-chef:latest-rust-alpine docker digest to c7496a3 (#62)([`4f71c3c`](https://github.com/kondanta/shepherd/commit/4f71c3c080e720ec7700e5d4012ef2069c1cf890))

- **deps:** update docker/dockerfile:1 docker digest to 87999aa (#57)([`f37d765`](https://github.com/kondanta/shepherd/commit/f37d76542e682fa130b4aa08adba27be05e45dd3))

- **deps:** update docker:cli docker digest to 9ba8e32 (#50)([`c279aad`](https://github.com/kondanta/shepherd/commit/c279aad158bea2b3721f97dfe72075967f112161))

- update dependencies (#61)([`b218b4c`](https://github.com/kondanta/shepherd/commit/b218b4c4bc2dd44609f8c23d77326559e12d10b0))

- bump dependencies (#54)([`9311270`](https://github.com/kondanta/shepherd/commit/9311270eafbef00787c25225a5c968dc0c09148c))

- **deps:** update deps (#48)([`28e86c0`](https://github.com/kondanta/shepherd/commit/28e86c044fbf82c24e4080efa9e9ff8f52e49c22))

- update Cargo.lock (#45)([`86f1d97`](https://github.com/kondanta/shepherd/commit/86f1d97ab9293e45d0b9bec25577b03078f0df6c))

- **deps:** update docker:cli docker digest to 17b5c23 (#44)([`62d3500`](https://github.com/kondanta/shepherd/commit/62d3500a1ee10489a3bf11f2007854b7d7888c79))

- update deps([`27d0b2a`](https://github.com/kondanta/shepherd/commit/27d0b2ad78263756e8f1a8058542ce994ee65e41))

- **deps:** update lukemathwalker/cargo-chef:latest-rust-alpine docker digest to b4cf4bd (#43)([`6f1fdb9`](https://github.com/kondanta/shepherd/commit/6f1fdb93b50fc535a89d1aab6eaabac41d0b32af))

- update tokio([`9bb7033`](https://github.com/kondanta/shepherd/commit/9bb7033193447cd557c7371c08c14177dd922f3a))

- bump shepherd version on Cargo.toml([`1208a25`](https://github.com/kondanta/shepherd/commit/1208a250405fb2c5f9aa0db299c0051daa830fe8))

- do not leak full system path([`845dce4`](https://github.com/kondanta/shepherd/commit/845dce4ce014c8a0730bbec7d3479f34adc1215e))

- update deps([`d3320b0`](https://github.com/kondanta/shepherd/commit/d3320b08458c2b4bb61aa228c661cf55f5fce9fe))

- **deps:** update docker:cli docker digest to 2efe7c8 (#41)([`749b9ef`](https://github.com/kondanta/shepherd/commit/749b9ef14ff158083e316e2aec88ff00aa5ecbfb))

- **deps:** update rust crate clap to v4.6.1 (#40)([`1bd9aa0`](https://github.com/kondanta/shepherd/commit/1bd9aa0b2770d48e056bdaae0ac9c8684177427a))

- **deps:** update rust crate tokio to v1.52.0 (#39)([`921f7b2`](https://github.com/kondanta/shepherd/commit/921f7b2f2ce6c305c52520311764d1aedd8c3054))

- **deps:** update rust crate axum to v0.8.9 (#38)([`2f291e9`](https://github.com/kondanta/shepherd/commit/2f291e9538e5d1d355f1769235753f35f3f80376))

- **deps:** update cargo deps([`b820744`](https://github.com/kondanta/shepherd/commit/b820744a876db7d926ca64d927891fe06bbe3f10))

- **deps:** update rust crate tokio to v1.51.1 (#37)([`a5d71df`](https://github.com/kondanta/shepherd/commit/a5d71df7c7a5fb1976e3cffc1ffaae10164f5556))

- **deps:** update docker/dockerfile:1 docker digest to 2780b5c (#35)([`a0f3e9f`](https://github.com/kondanta/shepherd/commit/a0f3e9f3ac0c8f88547225f3a78f7e75fe289824))

- **deps:** update docker:cli docker digest to 0befd75 (#36)([`21496c5`](https://github.com/kondanta/shepherd/commit/21496c51bf9a085c5991e704e0f471ab22ea743f))

- **deps:** update rust crate arc-swap to v1.9.1 (#34)([`ad5aaec`](https://github.com/kondanta/shepherd/commit/ad5aaece82b4d8dfdfdfd3c80d2599cbe209cfaf))

- **deps:** update rust crate tokio to v1.51.0 (#32)([`4fdfde0`](https://github.com/kondanta/shepherd/commit/4fdfde0b80165b07df1b234cda4b9fc8ea5ac022))

- **deps:** update docker:cli docker digest to 18f5ab0 (#33)([`381eb5c`](https://github.com/kondanta/shepherd/commit/381eb5c522009856db101f19e16d9f4f1202b8bc))

- check absolute path for root([`1feee72`](https://github.com/kondanta/shepherd/commit/1feee722664f08999b83afc54da1eaeb6f834e7e))

- **deps:** update hmac and sha2([`4c8f334`](https://github.com/kondanta/shepherd/commit/4c8f3346536f426675b92dbd17e6b792a06292e2))

- update cargo ([`0a30b09`](https://github.com/kondanta/shepherd/commit/0a30b093526c59cdda9d9b3979cdbfc6ead194c7))

- **deps:** update lukemathwalker/cargo-chef:latest-rust-alpine docker digest to 5b2b5c6 (#30)([`c45586b`](https://github.com/kondanta/shepherd/commit/c45586be13bc5cb0f6858b17fc80e4aa20112296))

- **deps:** update docker:cli docker digest to 70303ed (#29)([`84c8c31`](https://github.com/kondanta/shepherd/commit/84c8c3139af05840659e9d2266a0c0696b7a403e))

- **deps:** update rust crate arc-swap to v1.9.0 (#27)([`d4da489`](https://github.com/kondanta/shepherd/commit/d4da489f041b62314c6465d5b5284c5fc5ef0acb))

- **deps:** update rust crate opentelemetry-otlp to v0.31.1 (#26)([`d3aaab9`](https://github.com/kondanta/shepherd/commit/d3aaab9a11f513905cdfca48b7e4fa9449099d4c))

- bump shepherd version([`44c16d6`](https://github.com/kondanta/shepherd/commit/44c16d6097b0d1c809a951d4cd4866d74f86bc29))

- **deps:** update docker/dockerfile:1 docker digest to 4a43a54 (#24)([`28203e5`](https://github.com/kondanta/shepherd/commit/28203e516d445258925a8bab705d39503835bdeb))

- run just fix([`2a09e5d`](https://github.com/kondanta/shepherd/commit/2a09e5d45b8bbac6300e033393d90efab649b43f))

- bump cargo([`c8c2723`](https://github.com/kondanta/shepherd/commit/c8c2723aeba6754a839681682a0b4f9e9036f96e))

- **deps:** pin dependencies (#23)([`12f77b5`](https://github.com/kondanta/shepherd/commit/12f77b52926264cef97fe5570ba97ccffc219d9b))

- update reqwest([`4845327`](https://github.com/kondanta/shepherd/commit/484532786f1d1ec7775df5b1ffa867676cfe2175))

- ignore .env([`6762d72`](https://github.com/kondanta/shepherd/commit/6762d72cdfe60d4e9e346672e4893f4a9851d247))

- update rustfmt([`bc4a40e`](https://github.com/kondanta/shepherd/commit/bc4a40e49de7f579e5c535001c4fc113711b726d))

- update cargo.lock([`75b6817`](https://github.com/kondanta/shepherd/commit/75b681792616ac609b37411d9a50f1c213159031))

- fix fmt([`105c3aa`](https://github.com/kondanta/shepherd/commit/105c3aa0b43530924938aeb9510b788e961e478a))

- update cargo([`85713ee`](https://github.com/kondanta/shepherd/commit/85713eee0f247c5224a3dd63e08a327fb5e89282))

- **deps:** update rust crate clap to v4.5.61 (#18)([`47530af`](https://github.com/kondanta/shepherd/commit/47530affa9ec3a25b97f27f104d0c6def2267866))

- **deps:** update rust crate tempfile to v3.27.0 (#17)([`2032d9e`](https://github.com/kondanta/shepherd/commit/2032d9e31e7e99a9f9687480cc6364971de6c7f3))

- **deps:** update rust crate which to v8.0.2 (#16)([`7f41f2f`](https://github.com/kondanta/shepherd/commit/7f41f2fd493ceae162b10df495e2e6b5ad09cfd5))

- ignore test folder([`d0b3f3c`](https://github.com/kondanta/shepherd/commit/d0b3f3c1917ef97b423739b8272cf9866aa72eb1))

- add endpoint for listing managed services([`d337a38`](https://github.com/kondanta/shepherd/commit/d337a388ece466f710a07f5e2accfa3874a232a5))

- **cargo:** update dependencies([`cd72715`](https://github.com/kondanta/shepherd/commit/cd72715bf118b86cec566ac7e0c0508422813d93))

- **deps:** update rust crate tokio to v1.50.0 (#15)([`4e23cb6`](https://github.com/kondanta/shepherd/commit/4e23cb6d168462b5aea955d9e806c06aa79cc03a))

- **deps:** update rust crate tokio-macros to v2.6.1 (#14)([`9f1bc4d`](https://github.com/kondanta/shepherd/commit/9f1bc4d8720363e468e719456a21e9b497d34860))

- **deps:** update rust crate tempfile to v3.26.0 (#13)([`621ea7f`](https://github.com/kondanta/shepherd/commit/621ea7f882389db4345c2d4d5d69f905d6e0b6d9))

- **deps:** update rust crate clap to v4.5.60 (#12)([`25b4149`](https://github.com/kondanta/shepherd/commit/25b4149ecc20b098e3621ed358520f6a1ac9b7cb))

- **deps:** update rust crate arc-swap to v1.8.2 (#11)([`7153917`](https://github.com/kondanta/shepherd/commit/71539170397a89d69a81451399d0cef49107f90b))

- **deps:** update rust crate env_logger to v0.11.9 (#10)([`e0f0c0b`](https://github.com/kondanta/shepherd/commit/e0f0c0b70b7c67b3a436fd7a514b4a765babe7ac))

- **deps:** update rust crate clap to v4.5.58 (#9)([`59c720b`](https://github.com/kondanta/shepherd/commit/59c720bc9ef060e81f186d574ff0e49fb0def4bc))

- **deps:** update rust crate tempfile to v3.25.0 (#8)([`9152f61`](https://github.com/kondanta/shepherd/commit/9152f61e0cb1210b6ec11526d9e30a6477db8ced))

- **deps:** update rust crate clap to v4.5.57 (#2)([`23bfd10`](https://github.com/kondanta/shepherd/commit/23bfd1053041ddb33b003bc797d75ffa8ff8cf14))

- **deps:** update rust crate arc-swap to v1.8.1 (#7)([`e280881`](https://github.com/kondanta/shepherd/commit/e2808813992365115cb1a55be01f2375108f9543))

- **env:** add .env ([`08003e2`](https://github.com/kondanta/shepherd/commit/08003e2b2f04133c0cb89be449e82b923e1f8b18))

- **cargo:** update cargo.lock([`2953baf`](https://github.com/kondanta/shepherd/commit/2953bafc42989deb374f3ecc37df7d2086302758))

- **deps:** pin rust crate tempfile to =3.24.0 (#4)([`91a5ca4`](https://github.com/kondanta/shepherd/commit/91a5ca4507ecd60cb9c389ae3ed716af021a8bd4))

- add/remove dependencies([`e90a3bd`](https://github.com/kondanta/shepherd/commit/e90a3bdac2f4fb1ebfcab6b7e231ab561e53cb41))

- **clap:** bump version([`50843b5`](https://github.com/kondanta/shepherd/commit/50843b54f0a03e5ac37c2ff5bc857248e7f42254))

- bump clap([`341b87a`](https://github.com/kondanta/shepherd/commit/341b87a93508a67e9b21df0961b226238197f0fc))

- init project properly([`918f723`](https://github.com/kondanta/shepherd/commit/918f723da69d458a0559b768ee2a0a8a5c047323))



