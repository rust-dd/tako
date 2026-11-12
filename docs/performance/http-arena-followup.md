# HTTP Arena: további protokolloptimalizálás

Vizsgálat és helyi mérések: 2026. november 11–12.
Kiinduló commit: `28a1e2c7b9e22fac5d0c74444b5c99da17b7acd3`.
Ez az [előző vizsgálat](http-arena.md) utáni módosításokat méri.

A PR előkészítésekor a módosításokat a `main`
`e7995eb4c9c95f815b629fbbb9f62931a5c3fcf1` commitjára vezettük át.
Az alábbi teljesítményszámok és forráshash-ek a korábbi mérési forráspárt
írják le; az új alaphoz képest külön teljesítménymérés nem történt.

A közös HTTP-kiszolgálási útvonal, a WebSocket-küldés, a gRPC Content-Type
ellenőrzése és a tömörítési plugin kapott célzott optimalizálást. A helyi
mikromérésekben a kis gRPC-kérés feldolgozási ideje 10–15%-kal csökkent,
a kis WebSocket-üzenetek memóriabeli echo-teljesítménye 5–6%-kal nőtt.
A HTTP hálózati kontrollokból egyelőre nem igazolható egyértelmű gyorsulás.
Új hivatalos arénás vagy teljes gateway-mérés nem futott.

## A közzétett eredmények értelmezése

Az aréna adaptere továbbra is Tako 2.4.2-t használ; a méréskori helyi workspace
2.5.0.
Így a nyilvános számok sem az előző optimalizálásokat, sem ezt a módosítást
nem mérik. A `tuned=0` nézethez tartozó közzétett minták:

| Profil | Kapcsolatok | Tako kérés/s, illetve WS üzenet/s |
|---|---:|---:|
| H1 baseline | 4096 | 3 282 244 |
| H1 pipelined | 4096 | 22 398 380 |
| JSON tömörítéssel | 4096 | 211 108 |
| H2 TLS | 256 | 740 669 |
| H2 cleartext | 256 | 744 038 |
| H3 baseline | 64 | 540 317 |
| gRPC unary | 256 | 578 019 |
| gRPC unary TLS | 256 | 622 537 |
| WebSocket echo | 4096 | 4 062 111 |
| WebSocket pipeline | 4096 | 6 007 904 |
| H2 gateway | 512 | 77 411 |
| H3 gateway | 64 | 83 042 |

Források: [aktuális aréna](https://http-arena.com/#tuned=0),
[adapterfüggőségek](https://github.com/MDA2AV/HttpArena/blob/main/frameworks/tako/Cargo.toml),
[nyers Tako-eredmények](https://github.com/MDA2AV/HttpArena/blob/main/site/data/results/tako.json).
A számok eltérő munkaterheléseket jelentenek, egymással közvetlenül nem összehasonlíthatók.

A gatewayben Caddy zárja le a H2/H3-kapcsolatokat, és HTTP/1.1-en küldi tovább
a dinamikus kéréseket. A H2 gateway 512 kapcsolatos mérésében Caddy 3155%,
Tako 853% CPU-t használt a szolgáltatásonként 32 logikai CPU-s keretből.
Ez elsősorban proxyoldali korlátra utal. A tömörítési javítás az upstream
kiszolgálásban hasznosulhat, de ebből teljes gateway-gyorsulás nem következik.
[Konfiguráció és mérési korlátok](http-arena-gateway.md),
[Caddy-konfiguráció](https://github.com/MDA2AV/HttpArena/blob/main/frameworks/tako-gateway/proxy/Caddyfile).

## A módosítások és a megőrzött viselkedés

- **HTTP/1 TLS, HTTP/2 és HTTP/3:** a kapcsolati metaadatok akkor kerülnek
  a request extension mapbe, ha az extractor vagy a teljes middleware-pipeline
  használhatja őket. A puszta handler elkerüli a felesleges bejegyzések
  létrehozását; a szükséges bejegyzések újrahasználhatják a meglévő tárolót.
  A compio TLS-kiszolgáló is a közös HTTP service-t használja.
  A peer, TLS, ALPN, SNI, saját extensionök, HEAD és fallback viselkedését
  regressziós tesztek ellenőrzik.
- **WebSocket:** az inherent `send` egy zárolás alatt ellenőrzi a küldhetőséget
  és helyezi sorba az üzenetet. Sorba állítás előtti megszakításnál az új üzenet
  nem kerül elküldésre; flush közbeni megszakításnál a már elfogadott frame
  megmarad a sorban. Nincs duplikáció, a külön taskból küldés továbbra is flushol.
  A `Sink` API és a protokoll feldolgozása változatlan.
- **gRPC:** a paraméter nélküli `application/grpc` és `application/grpc+proto`
  értékeket kis- és nagybetűtől függetlenül, allokáció nélkül ellenőrizzük.
  Minden más érték az eddigi MIME parserhez kerül. A paraméterek, hibás
  headerek, framing, streaming és méretkorlátok viselkedése megmarad.
- **HTTP-válaszok tömörítése:** az Accept-Encoding feldolgozása kölcsönzött
  szeleteket és rögzített méretű tárolót használ. Az egyetlen DATA frame-ből
  álló bufferelt bodyhoz nem készül külön chunk-sor. Megmarad a szerver
  codec-preferenciája, a q-értékek, wildcard és ismételt codingok kezelése,
  valamint a töredezett body, trailer, pending és hibás stream viselkedése.
  A tömörítési szintek és az alapértelmezett kodekek nem változtak.

A metaadat-pooling ellenőrzése két regressziót is feltárt, amelyeket a végső
változat javít: az error handler többé nem láthat előző kérésből maradt route
paramétereket, és másik workerre kerülő kérés nem örökölhet régi body limitet.
Mindkét új teszt elbukott a javítás előtt és átment utána.

A QUIC-tesztkliensnél egy időzítésfüggő lezárás is előjött: a szerver a teljes
válasz mellett megszakíthatja az üres kérés fogadási irányát. A teszt és a
benchmark kizárólag a `finish()` alatti `RemoteTerminate(0)` esetet tolerálja;
a státusz, válaszbody és EOF ellenőrzése megmarad. A produkciós HTTP/3 body
lezárását nem módosítottuk. A korai válasz és a kérésirány megszakításának
szabályait az [RFC 9114, 4.1. szakasza](https://www.rfc-editor.org/rfc/rfc9114.html#section-4.1),
az ismeretlen hibakódok kezelését a
[8. szakasz](https://www.rfc-editor.org/rfc/rfc9114.html#section-8) írja le.

## Mérések

Környezet: macOS arm64, Rust 1.99, ugyanazon gépen futó kliens és szerver.
Mérés közben nem futott párhuzamos fordítás, teszt vagy másik benchmark.
A gép nem CPU-izolált, ezért a hálózati futások szórása érdemi korlát.
A [mérési adatok](http-arena-followup.json) minden felhasznált futást,
beállítást, bináris-hasht és részidő-becslést tartalmaznak. Nem hagytunk el
kiugró mérési köröket. Egy korábbi, fordítással átfedő H2-próba nem része
az alábbi, később külön lefuttatott A/B/A/B összehasonlításnak.

A mikromérés-binárisok a közös munkapéldányból készültek, a vizsgált komponens
régi és új forrásával, azonos harness és lockfile mellett. Nem teljes,
elkülönített repository-pillanatképek: a gRPC és WS időzített útvonalát nem
érintik a párhuzamosan készülő transportmódosítások; a két tömörítési bináris
azonos core artifactot használ. A későbbi módosítások ezeket az időzített
útvonalakat nem változtatták meg. A JSON a részletes származást is rögzíti.

### HTTP hálózati kontrollok

Külön négyszálas kliens- és szerver-runtime, 64 élő kapcsolat, bemelegítés,
státusz- és bodyellenőrzés. H1: kapcsolatonként 8192 kérés, öt kör/futás.
H2: 32 stream, 32 768 kérés/kapcsolat, három kör/futás.
H3: 32 stream, 8192 kérés/kapcsolat, három kör/futás.
Minden eset A/B/A/B sorrendben futott; a táblázat az összes kör mediánját adja.

| Profil | Előtte kérés/s | Utána kérés/s | Eltérés |
|---|---:|---:|---:|
| H1 keepalive | 184 635 | 184 784 | +0,08% |
| H1 pipeline, 16 kérés | 3 013 598 | 3 026 022 | +0,41% |
| H2 cleartext | 778 450 | 784 320 | +0,75% |
| H3 | 111 983 | 116 635 | +4,15% |

H1 és H2 lényegében változatlan. H3-nál az előtte tartomány 106 863–114 509,
az utána tartomány 109 888–124 739 kérés/s; az első pár utána futása lassabb,
a második gyorsabb volt. Emiatt a pozitív összesített eltérés még nem elég
egyértelmű gyorsulási állításhoz. A H1 harness a multithread szervert méri;
az aréna H1 adaptere per-thread szervert használ. A H3 mérése tartalmazza
a QUIC titkosítását. A TCP-s TLS-profilokat, a kapcsolatfelépítés költségét,
statikus fájlokat, adatbázist és a teljes arénás kevert terhelést nem mérjük.

### WebSocket memóriabeli echo

Három A/B pár, optimalizált build. A mérés a már beolvasott frame-ek
`recv` + `send` feldolgozását méri, hálózat és flush nélkül. Soronként a három
futás mediánja szerepel; a kis üzeneteknél 8 192 000, 64 bájtnál 1 024 000,
1 KiB-nál 64 000 üzenet futásonként.

| Payload | Előtte üzenet/s | Utána üzenet/s | Eltérés |
|---|---:|---:|---:|
| 5 B | 24 223 401 | 25 451 550 | +5,07% |
| 64 B | 23 663 009 | 25 057 568 | +5,89% |
| 1 KiB | 11 055 668 | 10 870 090 | −1,68% |

Az 1 KiB-os mérés időzített része csak néhány milliszekundum, és nem mutat
előnyt. A kisebb üzenetek eredménye sem teljes WebSocket-hálózati gyorsulás.
A mérésben nincs zárolási versengés, backpressure, fragmentáció, szöveges
üzenet vagy külön taskból küldés; ezek helyességét a tesztek ellenőrzik.

### gRPC unary extractor

Az aréna két egész számot összeadó kéréséhez hasonló protobuf input,
nyilvános `GrpcRequest` extractor, request-építéssel együtt. Két A/B pár,
esetenként egy másodperc bemelegítés, három másodperc mérés, 50 Criterion-minta.
A benchmark csoportbeállításai felülírják a parancssor rövidebb időzítését.
A táblázat futásonkénti Criterion-időbecslések mediánját mutatja.

| Content-Type | Előtte ns | Utána ns | Időváltozás |
|---|---:|---:|---:|
| `application/grpc` | 222,32 | 199,40 | −10,31% |
| `application/grpc+proto` | 238,54 | 201,86 | −15,38% |
| `Application/Grpc+Proto` | 233,38 | 199,42 | −14,55% |
| `application/grpc; charset=utf-8` | 248,96 | 248,39 | −0,23% |

Külön, thread-local számláló allocatorral a már elkészített, statikus bodyt
használó paraméter nélküli kérés extractora **1 helyett 0 allokációt** végez.
Ebben a mérésben a request elkészítése és a `BodyLimit(None)` beállítása
a számláláson kívül van. Ez a Content-Type feldolgozás további megtakarítása
az előző framing-optimalizálás után, nem a teljes hálózati gRPC-kérés költsége.

### Tömörítés

Két A/B pár, esetenként egy másodperc bemelegítés, két másodperc célidő és
30 Criterion-minta. A buffered és negotiation esetekben a request építése
az időzítésen kívül történik; a dispatch és a tömörítés benne van. A buffered
payloadok előre elkészített JSON-adatok.
A méretcímkékhez tartozó tényleges bodyhossz 140, 1031 és 16 394 bájt;
a legkisebb body alapbeállítással még nem tömörül.
Az 512 KiB-os mixed eset előre elkészített, nyomtatható ASCII szöveget használ;
mindkét request felépítése és a handler bodymásolása is az időzítés része.

| Eset | Előtte µs | Utána µs | Időváltozás |
|---|---:|---:|---:|
| Negotiation: üres header, kontroll | 0,521 | 0,491 | −5,81% |
| Negotiation: gzip | 0,537 | 0,498 | −7,29% |
| Negotiation: böngészős, négy coding | 0,612 | 0,544 | −11,02% |
| Negotiation: q-paraméterek | 0,668 | 0,597 | −10,57% |
| Negotiation: 128 ismeretlen token | 4,484 | 2,297 | −48,77% |
| 140 B, identity | 0,643 | 0,605 | −5,86% |
| 140 B, gzip kérés | 0,682 | 0,630 | −7,51% |
| 140 B, Brotli kérés | 0,687 | 0,624 | −9,24% |
| 140 B, deflate kérés | 0,690 | 0,628 | −9,04% |
| 1031 B, identity | 0,668 | 0,602 | −9,92% |
| 1031 B, gzip | 7,001 | 6,879 | −1,75% |
| 1031 B, Brotli | 12,682 | 12,537 | −1,15% |
| 1031 B, deflate | 6,917 | 6,693 | −3,23% |
| 16 394 B, identity | 0,633 | 0,605 | −4,42% |
| 16 394 B, gzip | 17,974 | 17,430 | −3,03% |
| 16 394 B, Brotli | 26,504 | 24,145 | −8,90% |
| 16 394 B, deflate | 17,624 | 16,962 | −3,76% |
| 512 KiB vegyes request benchmark | 7772 | 7815 | +0,55% |

Az üres header kontrollja is 5,8%-kal elmozdult, és az első 16 KiB-os
Brotli-futás erősen szórt. Ezért a kis időeltérések teljes egészét nem lehet
az optimalizálásnak tulajdonítani. A nagy vegyes terhelés lényegében változatlan.
Az ismeretlen tokenes eset a parser költségét emeli ki, nem tipikus HTTP-terhelés.

Az allokációs regressziós tesztek erősebb, időzítéstől független bizonyítékot
adnak: az `identity` negotiation korábbi **2 allokáció / 136 bájt** többlete
megszűnt, és 1024 ismeretlen token sem okoz további allokációt. Az egyetlen
frame-ből álló, tömörítési küszöb alatti body vizsgálatánál a korábbi
**1 allokáció / 128 bájt** chunk-sor is megszűnt. Ezek a változatlan alap
dispatchhez képesti többletek, nem a teljes HTTP-kérés allokációszámai.

## Ellenőrzés és újrafuttatás

A méréskori forráson a teljes workspace tesztjei sikeresek voltak:
a bővített Tokio-konfigurációban
805 teszt átment, 16 ignored; `--all-features` mellett 779 átment, 42 ignored.
A compio HTTP/3 kliensmódosításának három integrációs tesztje külön is átment.
A csak `http3` feature-rel futtatott két érintett integrációs teszt is sikeres.
Mindkét feature-konfiguráció tests/benches Clippy ellenőrzése sikeres volt
`-D warnings` mellett. A nightly rustfmt és a `git diff --check` is sikeres.

A méréskori Tokio feature-lista:
`ws,sse,proxy-protocol,udp,per-thread,tls,http2,http3,webtransport,plugins,signals,multipart,simd,protobuf,grpc,ip-filter,hmac-signature,json-schema,zstd,typed-header,zero-copy-extractors,jwt-simple,file-stream`.

A friss `main`-re átvezetett PR-verzió a repository
`scripts/check-workspace.sh test` parancsával is átment mindhárom profilban:
default **502**, Tokio-rich **870**, all-features **820** sikeres teszt;
az ignored tesztek száma rendre 13, 44 és 43. Ez az integrált változat
helyességi ellenőrzése, és nem módosítja a korábbi teljesítménymérés eredményeit.
A `scripts/check-workspace.sh clippy` mindhárom profilja, az all-features
benchmarkok szigorú Clippy-ellenőrzése és a nightly rustfmt is sikeres volt.

```sh
CONNECTIONS=64 PIPELINE=1 REQUESTS=8192 ROUNDS=5 WORKERS=4 \
  cargo bench -p tako-rs-server --bench http1
CONNECTIONS=64 PIPELINE=16 REQUESTS=8192 ROUNDS=5 WORKERS=4 \
  cargo bench -p tako-rs-server --bench http1
CONNECTIONS=64 STREAMS=32 REQUESTS=32768 ROUNDS=3 WORKERS=4 \
  cargo bench -p tako-rs-server --features http2,http3 --bench http2
CONNECTIONS=64 STREAMS=32 REQUESTS=8192 ROUNDS=3 WORKERS=4 \
  cargo bench -p tako-rs-server --features http2,http3 --bench http3
cargo test -p tako-rs-core --release --features ws --lib \
  ws::socket::benchmarks::buffered_echo_throughput -- --exact --ignored --nocapture
cargo bench -p tako-rs-core --features grpc --bench grpc_decode -- unary_sum
cargo bench -p tako-rs-plugins --features plugins --bench compression -- \
  --warm-up-time 1 --measurement-time 2 --sample-size 30 --noplot
```

Összehasonlításhoz azonos harness, lockfile és fordító szükséges az alapcommit
és az új forrás mellett; külön megőrzött binárisokat futtassunk felváltva.
Az arénás hatás következő érdemi ellenőrzése dedikált Linux-gépen, a friss
Tako-forrásra állított adapterrel végzett teljes protokollmérés. A gatewayt
és a proxy CPU-elosztását külön kísérletként kell mérni.
