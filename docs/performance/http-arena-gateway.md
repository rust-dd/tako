# HttpArena gateway: mérési megállapítások

Ellenőrizve: 2026. október 24. Az alábbi számok az HttpArena `main` ágán
közzétett eredmények; ebben az elemzésben nem futott új gateway-mérés.
Az adapter `Cargo.lock` fájlja a Tako összes komponensét **2.4.2** verzióra
rögzíti, míg a vizsgált helyi kiadás **2.5.0**.
[Adapterfüggőségek](https://github.com/MDA2AV/HttpArena/blob/main/frameworks/tako/Cargo.lock#L2650).

A gateway eredményeiben elsősorban a Caddy oldali korlát látszik. A két gateway
profil alapbeállításban 16–16 fizikai magot és azok SMT-párjait adja a proxynak,
illetve a Takónak: szolgáltatásonként 32 logikai CPU-t.
[CPU-elosztás](https://github.com/MDA2AV/HttpArena/blob/main/frameworks/tako-gateway/compose.gateway.yml#L6).

| Profil | Kapcsolatok | Kérés/s | Caddy CPU | Tako CPU |
|---|---:|---:|---:|---:|
| `gateway-64` | 512 | 77 411 | 3155% | 853% |
| `gateway-64` | 1024 | 68 883 | 2955% | 867% |
| `gateway-h3` | 64 | 83 042 | 2668% | 781% |
| `gateway-h3` | 256 | 76 483 | 2675% | 715% |

Forrás: [közzétett gateway-eredmények](https://github.com/MDA2AV/HttpArena/blob/main/site/data/results/tako.json#L355).

A Docker CPU-százalékában 100% egy logikai CPU idejének felel meg. Így a
512 kapcsolatos H2-mérésben Caddy a 3200%-os keretének körülbelül 98,6%-át,
Tako 26,7%-át használja. Ez mintavételezett átlag, nem pontos, futásra integrált
CPU-idő: a mérő `docker stats --no-stream` mintákból számol. Az SMT-szálak
számából nem következik azonos számú független fizikai mag teljesítménye.
Az adatok a proxy terhelésére utalnak, de a gyorsulás mértékét csak új mérés
igazolhatja; a hálózati és I/O-korlátokat sem zárják ki.
[Mintavételezés és átlagolás](https://github.com/MDA2AV/HttpArena/blob/main/scripts/lib/stats.sh#L93).

A `production-stack` ugyanezt erősíti: 256 kapcsolatnál 54 377 kérés/s,
edge 2828%, Tako 141%; 1024 kapcsolatnál 47 171 kérés/s, edge 2960%,
Tako 224%. Itt az edge kerete 30, a szerveré 24 logikai CPU.
[Eredmények](https://github.com/MDA2AV/HttpArena/blob/main/site/data/results/tako.json#L659),
[elosztás](https://github.com/MDA2AV/HttpArena/blob/main/frameworks/tako-gateway/compose.production-stack.yml).

Caddy végzi a TLS és a HTTP/2 vagy HTTP/3 lezárását, és közvetlenül szolgálja ki
a statikus fájlokat. A dinamikus kéréseket HTTP/1.1 keepalive kapcsolatokon
továbbítja a Tako `serve_per_thread` szerveréhez, a 8080-as portra. Emiatt a
Tako HTTP/2- és HTTP/3-transzportjának javítása közvetlenül nem gyorsítja ezt
a gateway-konfigurációt.
[Proxykonfiguráció](https://github.com/MDA2AV/HttpArena/blob/main/frameworks/tako-gateway/proxy/Caddyfile#L12),
[szerverindítás](https://github.com/MDA2AV/HttpArena/blob/main/frameworks/tako/src/main.rs#L199).

A munkaterhelés 30% statikus fájl, 35% JSON, 20% lekérdezési paraméterek
összeadása és 15% adatbázis-lekérdezés. A Tako a dinamikus rész HTTP/1.1
kiszolgálását, útválasztását, paraméterfeldolgozását, JSON-szerializációját és a JSON
útvonalak alapértelmezett Brotli-tömörítését tudja javítani. A DB-driver és az
adapter alkalmazáskódja szintén része a mért időnek. A 2.5.0 tömörítési pluginja
már megosztja a konfigurációt kérésenkénti teljes másolás helyett; ennek gateway
hatására még nincs új eredmény.
[Munkaterhelés](https://github.com/MDA2AV/HttpArena/blob/main/requests/gateway-64-uris.txt),
[JSON-kezelő és plugin](https://github.com/MDA2AV/HttpArena/blob/main/frameworks/tako/src/main.rs#L105),
[helyi plugin](../../tako-rs-plugins/src/plugins/compression/plugin.rs).

A következő kísérlet helyi HttpArena-munkapéldányban fusson. Először azonos
adapterrel és CPU-elosztással hasonlítsuk össze a 2.4.2-t a vizsgált Tako-builddel.
Ezután, ugyanazzal a builddel, külön kísérlet legyen a 16/16 és 24/8 fizikai
magos proxy/szerver elosztás. Az alábbi CPU-azonosítók az HttpArena gépének
topológiáját feltételezik; más Linux-gépen a teljes magokat és SMT-párjaikat
együtt kell kiosztani.

```sh
./scripts/validate.sh tako-gateway
./scripts/benchmark.sh tako-gateway gateway-64
./scripts/benchmark.sh tako-gateway gateway-h3
PROXY_CPUSET='0-23,64-87' SERVER_CPUSET='24-31,88-95' \
  ./scripts/benchmark.sh tako-gateway gateway-64
PROXY_CPUSET='0-23,64-87' SERVER_CPUSET='24-31,88-95' \
  ./scripts/benchmark.sh tako-gateway gateway-h3
```

Ezekhez nem kell `--save`, publikálás vagy upstream módosítás. A kérés/s mellett
szolgáltatásonkénti CPU-t, memóriát és sikeres válaszokat is hasonlítsunk össze.
A profilok kapcsolatonként 32 streamet, 5 másodperces futásokat és három közül
a legjobb eredményt használják; a rövid, legjobb futást választó mérés miatt
érdemes az összehasonlítást ismételt A/B futásokkal ellenőrizni.
[H2 paraméterek](https://github.com/MDA2AV/HttpArena/blob/main/site/content/docs/test-profiles/gateway/gateway-h2/implementation.md#parameters),
[H3 paraméterek](https://github.com/MDA2AV/HttpArena/blob/main/site/content/docs/test-profiles/gateway/gateway-h3/implementation.md#parameters).

A H2-mérő a `p99_latency` mezőbe valójában a h2load által közölt **maximumot**
írja, mert az adott kimenet nem ad percentilist. Ez a H2 gateway, a közvetlen
H2 és a gRPC profilok értelmezésénél is számít: ezekből a számokból valódi
p99-javulás nem állapítható meg.
[Ellenőrzött parser](https://github.com/MDA2AV/HttpArena/blob/main/scripts/lib/tools/h2load.sh#L251).
