# HttpArena: HTTP/2, HTTP/3 és gRPC

Vizsgálat: 2026. október 24. Kiinduló commit: `0fb91ee6`.
Az aréna közzétett adaptere Tako 2.4.2-t használ; a helyi kiindulópont 2.5.0.
A rangsor ezért önmagában nem írja le a jelenlegi repository teljesítményét.
[HttpArena](https://http-arena.com/frameworks/tako/),
[adapterfüggőségek](https://github.com/MDA2AV/HttpArena/blob/main/frameworks/tako/Cargo.lock).

Három szűk körű módosítás készült. A másolás és a megosztott referenciaszámláló
terhelésének csökkenése teszttel igazolt. A helyi hálózati mérésekből **nem
állapítható meg HTTP/2- vagy HTTP/3-RPS-javulás**. Új hivatalos arénás mérés
nem futott.

## Elkészült módosítások

### Kompakt Hyper service future

A Tokio h2c és TLS kiszolgáló közös
[`http_service`](../../tako-rs-core/src/server_support/http.rs) függvényt használ.
Az egyszerű route közvetlenül a handler future-jét adja tovább, a teljes
middleware-pipeline külön heap-allokációt kap. Így az egyszerű kérésekhez
nem kell a nagyobb pipeline teljes future-állapotát minden streamhez tárolni.

Ugyanazzal a fordítóval mért `size_of_val` eredmény: a régi service future
**3424 bájt**, az új **136 bájt**. Ez egy future mérete, nem a teljes kérés
memóriaigénye vagy egy RSS-mérés. A middleware-t használó ágon az új box
külön allokáció; a fenti arány arra nem alkalmazható teljes megtakarításként.

A h2c-kérések kapcsolatonként külön `Arc`-számlálót használnak. A közös
endpoint-router számlálóját nem minden aktív stream módosítja. A TLS-ágon
ez a kapcsolatonkénti megosztás már létezett; ott a future mérete változik.
A peer, a TLS/ALPN/SNI metaadatok, a middleware, a HEAD és a fallback
viselkedését [regressziós tesztek](../../tako-rs-core/src/server_support/http/tests.rs)
ellenőrzik. Száz várakozó teljes pipeline mellett a közös router referenciaszáma
továbbra is kettő.

### HTTP/3: kapcsolatonkénti router-hivatkozás

A Tokio HTTP/3 kapcsolatok egy külső `Arc`-ot, a compio kapcsolatok egy `Rc`-t
tartanak. A request taskok ezt másolják. A különböző kapcsolatok kérései így
nem ugyanazt az endpoint-szintű atomikus számlálót növelik és csökkentik.
Ennek ára kapcsolatonként egy kis allokáció. A WebTransport átadásakor egyszer
még szükséges a belső `Arc` másolása.

A [valódi QUIC-kapcsolatot használó teszt](../../tako-rs-server/src/server_h3/connection/tests.rs)
nyolc egyidejű, visszatartott kérésnél **10 helyett 2** közös router-hivatkozást
mért. A kapcsolaton belül továbbra is van számlálás; a több kapcsolat közötti
versengést szüntetjük meg ezen az útvonalon.

### gRPC: teljes frame-ek másolás nélküli dekódolása

A [`FrameDecoder`](../../tako-rs-core/src/grpc/framing.rs) megőrzi a beérkező
`Bytes` chunkot. A benne teljesen elférő üzenetet ebből választja le, ahelyett,
hogy előbb egy `BytesMut` bufferbe másolná. Csak a több DATA chunkra töredezett
aktuális frame kerül az összeállító bufferbe. Az utána következő teljes frame-ek
ismét megoszthatják az eredeti chunkot.

A változás megtartja az üzenetméret-korlátot, a tömörített frame elutasítását,
a hibák utáni lezárást és az eddigi unary szemantikát. Tizenkét új
[teszt](../../tako-rs-core/src/grpc/tests/framing.rs) ellenőrzi a megosztott
memóriát, a töredezett inputot, a több üzenetet, a trailer/pending eseteket és
a hibákat. A három pointer-megosztási regressziós teszt a régi implementáción
elbukott.

## Helyi mérések

Környezet: macOS, arm64, Rust 1.99; kliens és szerver ugyanazon, más munkára is
használt gépen. A hálózati benchmark külön négyszálas kliens- és szerver-runtime-ot
használ, élő kapcsolatokkal, bemelegítéssel és válaszellenőrzéssel. A mérés
nem CPU-izolált, nem az aréna Linux-gépe, és nem tartalmaz teljes alkalmazásos
gateway- vagy gRPC-terhelést.

| Hálózati profil | Régi medián kérés/s | Új medián kérés/s | Eltérés | Értelmezés |
|---|---:|---:|---:|---|
| H2, 64 kapcsolat × 32 stream | 425 318 | 415 202 | −2,4% | Nagy szórás; nincs igazolt RPS-javulás |
| H3, 64 kapcsolat × 32 stream | 226 858 | 222 532 | −1,9% | Nagy szórás; nincs igazolt RPS-javulás |

Mindkét összehasonlítás A/B/A/B sorrendben futott. H2-nél futásonként három
mérési kör és kapcsolatonként 32 768 kérés, H3-nál öt kör és 2048 kérés volt.
A rövid első H2-próbákat nem használtuk a fenti összesítéshez. A felhasznált
körök és a beállítások a [nyers eredményekben](http-arena-local.json) szerepelnek;
kiugró köröket nem hagytunk el.

A gRPC Criterion microbenchmark a nyilvános extractor útvonalát méri:

| gRPC decode eset | Régi idő | Új idő | Eltérés |
|---|---:|---:|---:|
| Egybefüggő, 32 B `Bytes` mező | 214,21 ns | 197,97 ns | −7,6% |
| Egybefüggő, 1 KiB `Bytes` mező | 224,67 ns | 193,10 ns | −14,1% |
| Egybefüggő, 64 KiB `Bytes` mező | 1374,5 ns | 194,55 ns | −85,8% |
| Töredezett, 32 B | 293,54 ns | 303,77 ns | +3,5% |
| Töredezett, 1 KiB | 334,15 ns | 333,85 ns | −0,1% |
| Töredezett, 64 KiB | 1774,6 ns | 1789,5 ns | +0,8% |
| 128 kis üzenet egy chunkban | 1894,8 ns | 2021 ns | +6,7% |

Ez helyi mérési eredmény, nem általános gyorsulási ígéret. A kis, kötegelt és
töredezett eseteknek költsége is lehet; a kontrollismétlés zajos volt. A nagy,
egybefüggő `Bytes` mező előnye közvetlenül a megszüntetett másolásból származik.

Az aréna [gRPC-adaptere](https://github.com/MDA2AV/HttpArena/blob/main/frameworks/tako-grpc/src/main.rs)
két `int32` mező összeadását használja. Ehhez külön `unary_sum` benchmark készült.
Közben a benchmarktól független fordítások is futottak a gépen, és a három
helyi régi/új összehasonlítás erősen szórt. **A `unary_sum` mérésből nincs
értékelhető teljesítményeredmény**, így az arénás kis gRPC-kérés gyorsulása sem igazolt.
A 64 KiB-os eredmény nem helyettesíti ezt a mérést.

Külön, instrumentált allocatorral a két egész mezős kérés dekódolása
**2 allokáció / 50 bájtról 0 allokáció / 0 bájtra** csökkent. Ezt egy
bemelegítés után 1000 extract átlagán mértük, current-thread runtime-on.
A request és a megosztott bemeneti `Bytes` elkészítése kívül esett a számláláson;
ez a decoder/extractor szakasz eredménye, nem a teljes hálózati kérésé.
Az allokációcsökkenés igazolt, a belőle származó arénás RPS-előny még mérendő.

## Újrafuttatás

Az új harness-eket a CI feature-ökkel együtt lefordítja. A hálózati harness-ek
Tokiót használnak; futtatásukhoz ne kapcsoljuk be a `compio` feature-t.

```sh
CONNECTIONS=64 STREAMS=32 REQUESTS=32768 ROUNDS=3 WORKERS=4 \
  cargo bench -p tako-rs-server --features http2 --bench http2
CONNECTIONS=64 STREAMS=32 REQUESTS=2048 ROUNDS=5 WORKERS=4 \
  cargo bench -p tako-rs-server --features http3 --bench http3
cargo bench -p tako-rs-core --features grpc --bench grpc_decode
cargo bench -p tako-rs-core --features grpc --bench grpc_decode -- unary_sum
```

Tiszta összehasonlításhoz ugyanazokat az új benchmark-forrásokat kell a
kiinduló commitból készített külön munkapéldányra is rátenni; az alapcommitban
még nem léteznek. Külön target könyvtárban, ugyanazzal a lockfile-lal és
fordítóval építsük meg a két binárist. Az időzítés alatt ne fordítsunk.
Dedikált Linux-gépen ismételt A/B/A/B futás és azonos CPU-kiosztás szükséges.
A teljes, middleware-es alkalmazást és a kis gRPC-kérést is külön mérjük.

## Gateway

A közzétett profilban Caddy zárja le a HTTP/2–3 kapcsolatokat, a dinamikus
upstream HTTP/1.1. A CPU-minták elsősorban a proxy telítődését mutatják;
512 H2-kapcsolatnál Caddy 3155%, Tako 853% CPU-t használ a fejenként
32 logikai CPU-s keretből. Ezért itt nem indokolt találomra Tako H2/H3
beállításokat módosítani. A források, a mérési korlátok és a 16/16 versus
24/8 fizikai magos kontrollkísérlet a
[külön gateway-elemzésben](http-arena-gateway.md) találhatók.

## Ellenőrzés

Sikeres volt a teljes workspace tesztfutása és a tests/benches Clippy ellenőrzése
`-D warnings` mellett mindkét konfigurációban:

- Tokio: `ws,sse,proxy-protocol,udp,per-thread,tls,http2,http3,webtransport,plugins,signals,multipart,simd,protobuf,grpc,ip-filter,hmac-signature,json-schema,zstd,typed-header,zero-copy-extractors,jwt-simple,file-stream`.
- Compio-t is tartalmazó konfiguráció: `--all-features`.

A nightly rustfmt és a `git diff --check` is sikeres. Az új benchmarkok
lefordultak. A website typecheck és tartalmi audit sikeres.
