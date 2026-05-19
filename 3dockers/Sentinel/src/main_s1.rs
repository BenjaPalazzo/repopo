/// sisar-download-s1
///
/// Lee /job/burst_list.json, verifica el cache en /archive, descarga los
/// bursts faltantes desde ASF, los ensambla en SAFEs con local2safe.py
/// y los comprime en .zip (layout ESA canónico para ISCE2).
///
/// Las órbitas precisas y el DEM corren en contenedores separados.
///
/// Credenciales: EARTHDATA_USER / EARTHDATA_PASS o ~/.netrc
///   (machine urs.earthdata.nasa.gov)

mod types;
use types::BurstList;

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use futures::future::try_join_all;
use tokio::sync::Semaphore;

// ── helpers ───────────────────────────────────────────────────────────────────

/// Ejecuta un subproceso heredando stdio. Falla si el exit code no es 0.
fn run(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<()> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let status = cmd
        .status()
        .with_context(|| format!("Failed to launch `{program}`"))?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "`{}` exited with status {}",
            program,
            status.code().unwrap_or(-1)
        ))
    }
}

/// Devuelve la ruta canónica de un burst en el archive.
/// Layout: {archive_root}/{slc}/{subswath}/{pol}/{index}.{ext}
fn archive_path(archive_root: &Path, slc: &str, sw: &str, pol: &str, idx: &str, ext: &str) -> PathBuf {
    archive_root
        .join(slc)
        .join(sw)
        .join(pol)
        .join(format!("{idx}.{ext}"))
}

// ── entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(e) = run_download().await {
        eprintln!("[sisar-download-s1] FATAL: {e:?}");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

async fn run_download() -> Result<()> {
    let job_dir      = PathBuf::from("/job");
    let archive_root = PathBuf::from("/archive");

    // Nivel de paralelismo: env var o default 4
    let concurrency: usize = env::var("DOWNLOAD_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    eprintln!("[sisar-download-s1] DOWNLOAD_CONCURRENCY={concurrency}");

    // ── 1. Parsear burst_list.json ────────────────────────────────────────────
    let burst_list_path = job_dir.join("burst_list.json");

    eprintln!("[sisar-download-s1] Leyendo burst_list.json …");
    let burst_list: BurstList = {
        let raw = fs::read_to_string(&burst_list_path)
            .context("Cannot read /job/burst_list.json")?;
        serde_json::from_str(&raw).context("Cannot parse burst_list.json")?
    };

    // ── 2. Detectar bursts faltantes en /archive ──────────────────────────────
    eprintln!("[sisar-download-s1] Verificando archive …");

    let mut missing: Vec<(String, PathBuf)> = Vec::new();

    for (slc, swaths) in &burst_list {
        let slc_upper = slc.to_uppercase();
        for (sw, pols) in swaths {
            let sw_upper = sw.to_uppercase();
            for (pol, bursts) in pols {
                let pol_upper = pol.to_uppercase();
                for (idx, entry) in bursts {
                    let tiff_dest = archive_path(&archive_root, &slc_upper, &sw_upper, &pol_upper, idx, "tiff");
                    let xml_dest  = archive_path(&archive_root, &slc_upper, &sw_upper, &pol_upper, idx, "xml");

                    if !tiff_dest.exists() {
                        missing.push((entry.data.clone(), tiff_dest));
                    }
                    if !xml_dest.exists() {
                        missing.push((entry.metadata.clone(), xml_dest));
                    }
                }
            }
        }
    }

    // ── 3. Descargar bursts faltantes en paralelo ─────────────────────────────
    if missing.is_empty() {
        eprintln!("[sisar-download-s1] Todos los bursts están en archive.");
    } else {
        eprintln!("[sisar-download-s1] Descargando {} archivo(s) desde ASF …", missing.len());

        let sem = Arc::new(Semaphore::new(concurrency));

        let tasks: Vec<_> = missing
            .into_iter()
            .map(|(url, dest)| {
                let sem = Arc::clone(&sem);
                tokio::spawn(async move {
                    let _permit = sem.acquire().await.expect("semaphore closed");
                    eprintln!("[sisar-download-s1]   GET {url}");
                    if let Some(parent) = dest.parent() {
                        fs::create_dir_all(parent)
                            .with_context(|| format!("Cannot create dir {}", parent.display()))?;
                    }
                    earthdata::download(&url, &dest)
                        .await
                        .with_context(|| format!("Failed to download {url}"))
                })
            })
            .collect();

        // Propaga el primer error que aparezca
        for result in try_join_all(tasks).await? {
            result?;
        }

        eprintln!("[sisar-download-s1] Todas las descargas completadas.");
    }

    // ── 4. Correr local2safe.py → produce SAFE(s) en /job/data ───────────────
    eprintln!("[sisar-download-s1] Ejecutando local2safe …");
    let data_dir = job_dir.join("data");
    fs::create_dir_all(&data_dir).context("Cannot create /job/data")?;

    run(
        "python3",
        &[
            "/usr/local/bin/local2safe",
            burst_list_path.to_str().unwrap(),
            "--all_anns",
            "--work_dir",
            data_dir.to_str().unwrap(),
        ],
        Some(&job_dir),
    )
    .context("local2safe failed")?;

    // ── 5. Comprimir cada directorio .SAFE en /job/data ──────────────────────
    eprintln!("[sisar-download-s1] Comprimiendo SAFEs …");
    zip_safes(&data_dir)?;

    eprintln!("[sisar-download-s1] Descarga Sentinel-1 completada → /job/data/*.zip listos para ISCE2.");
    Ok(())
}

// ── zip SAFEs ─────────────────────────────────────────────────────────────────

fn zip_safes(data_dir: &Path) -> Result<()> {
    for entry in fs::read_dir(data_dir).context("Cannot read /job/data")? {
        let entry = entry?;
        let path  = entry.path();
        if path.is_dir() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| anyhow!("Invalid SAFE dir name"))?;
            if name.ends_with(".SAFE") {
                let zip_name = name.replace(".SAFE", ".zip");
                let zip_path = data_dir.join(&zip_name);
                eprintln!("[sisar-download-s1]   Zipping {name} → {zip_name}");
                run(
                    "zip",
                    &["-r", "-9", zip_path.to_str().unwrap(), name],
                    Some(data_dir),
                )
                .with_context(|| format!("zip failed for {name}"))?;
                fs::remove_dir_all(&path)
                    .with_context(|| format!("Cannot remove {}", path.display()))?;
            }
        }
    }
    Ok(())
}

// ── earthdata sub-módulo (wrapper de la librería) ─────────────────────────────

mod earthdata {
    pub use earthdata_rs::download;
}
