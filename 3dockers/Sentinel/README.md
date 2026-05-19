# sisar/download-s1

Contenedor de descarga **Sentinel-1** del pipeline SISAR.

Responsabilidades:

1. Leer `/job/burst_list.json` y verificar el cache en `/archive`
2. Descargar los bursts faltantes desde ASF (paralelo, semáforo configurable)
3. Ensamblar SAFEs con `local2safe` (burst2safe)
4. Comprimir cada SAFE a `.zip` con layout ESA canónico

> **Fuera del alcance de este contenedor:**
> - **Órbitas precisas** → las descarga el contenedor de órbitas (corre antes de ISCE2).
> - **DEM** → lo descarga `sisar/download-dem`, que solo es necesario para MintPy.

El scheduler lanza este contenedor y espera exit code 0 antes de arrancar ISCE2.

---

## Archivos del repositorio

```
sisar-download-s1/
├── Dockerfile.s1            # Imagen S1 (FROM ghcr.io/osgeo/gdal:ubuntu-full-3.6.4)
├── Cargo.toml               # Un solo [[bin]]: sisar-download-s1
├── Cargo.lock
│
├── src/
│   ├── lib.rs               # earthdata_rs: cliente Earthdata
│   ├── main_s1.rs           # Binario S1: bursts → local2safe → zip
│   └── types.rs             # Tipos serde: BurstList
│
├── scripts/
│   └── local2safe.py        # Wrapper de burst2safe
│
├── bin/
│   ├── sisar-download-s1    # Binario Rust compilado
│   └── .netrc               # Credenciales opcionales
│
├── entrypoint.sh            # Entrypoint del contenedor
├── requirements.s1.txt      # Dependencias Python
├── docker-compose.yml       # Lanza download-s1 + download-dem en paralelo
└── env.example
```

## Qué cambió en esta iteración

| Archivo | Estado | Detalle |
|---|---|---|
| `src/main_s1.rs` | Modificado | Removido step de órbitas (`download_orbits` + llamada a `sentineleof`) |
| `Dockerfile.s1` | Modificado | Removido `sentineleof` del `pip install` y del smoke-test |
| `README.md` | Modificado | Refleja nuevo alcance |

## Entradas y salidas

### Entradas (mounts)

| Path | Descripción |
|---|---|
| `/job/burst_list.json` | Mapa SLC → subswath → pol → burst con rutas en `/archive` |
| `/job/burst_stitch/{YYYY-mm-dd}.json` | Un archivo por fecha de adquisición |
| `/archive/` | Cache de bursts (puede estar vacío) |

### Salidas

| Path | Descripción |
|---|---|
| `/job/data/*.zip` | SAFEs comprimidos (entrada directa de ISCE2) |
| `/archive/{SLC}/{SW}/{POL}/{IDX}.{tiff,xml}` | Bursts cacheados para futuros jobs |

## Variables de entorno

| Variable | Default | Descripción |
|---|---|---|
| `EARTHDATA_USER` | — | Usuario NASA Earthdata (requerido) |
| `EARTHDATA_PASS` | — | Contraseña NASA Earthdata (requerido) |
| `DOWNLOAD_CONCURRENCY` | `4` | Workers de descarga paralela |

## Cómo construir

```bash
# 1. Compilar el binario Rust
cargo build --release --bin sisar-download-s1
mkdir -p bin
cp target/release/sisar-download-s1 bin/

# 2. Build de la imagen
docker build -f Dockerfile.s1 -t sisar/download-s1:latest .
```

## Cómo ejecutar

```bash
docker run --rm \
  -v /path/to/job:/job \
  -v /path/to/archive:/archive \
  -e EARTHDATA_USER=miusuario \
  -e EARTHDATA_PASS=mipassword \
  -e DOWNLOAD_CONCURRENCY=4 \
  sisar/download-s1:latest
```

### Debug interactivo

```bash
docker run --rm -it --entrypoint /bin/bash \
  -v /path/to/job:/job \
  -e EARTHDATA_USER=miusuario \
  sisar/download-s1:latest
```
