/// earthdata-rs — NASA Earthdata download library
///
/// Handles EDL OAuth redirects, pre-signed S3 URLs, and HTTP 202 polling.
/// Credentials are read from EARTHDATA_USER/EARTHDATA_PASS env vars or ~/.netrc.
///
/// Adapted from the reference lib.rs provided in the project.
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use reqwest::{header::LOCATION, redirect::Policy, Client, StatusCode};
use url::Url;

const MAX_POLLS: u32 = 20;
const POLL_BASE_SECS: u64 = 3;

// ── Credentials ───────────────────────────────────────────────────────────────

struct Creds {
    user: String,
    pass: String,
}

/// Load credentials from env vars (`EARTHDATA_USER` / `EARTHDATA_PASS`)
/// or fall back to `~/.netrc`.
fn load_creds() -> Result<Creds> {
    if let (Ok(u), Ok(p)) = (env::var("EARTHDATA_USER"), env::var("EARTHDATA_PASS")) {
        return Ok(Creds { user: u, pass: p });
    }
    parse_netrc()
}

fn parse_netrc() -> Result<Creds> {
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .context("Cannot determine home directory")?;
    let path = Path::new(&home).join(".netrc");
    let contents =
        fs::read_to_string(&path).with_context(|| format!("Cannot read {}", path.display()))?;

    let tokens: Vec<&str> = contents.split_whitespace().collect();
    let mut i = 0;
    let mut in_machine = false;
    let mut user = None;
    let mut pass = None;

    while i < tokens.len() {
        match tokens[i] {
            "machine" => {
                i += 1;
                in_machine = i < tokens.len()
                    && (tokens[i].contains("earthdata.nasa.gov")
                        || tokens[i].contains("urs.earthdata.nasa.gov"));
            }
            "login" if in_machine => {
                i += 1;
                user = tokens.get(i).map(|s| s.to_string());
            }
            "password" if in_machine => {
                i += 1;
                pass = tokens.get(i).map(|s| s.to_string());
            }
            "default" if in_machine => break,
            _ => {}
        }
        i += 1;
    }

    Ok(Creds {
        user: user.ok_or_else(|| anyhow!("No login found in {}", path.display()))?,
        pass: pass.ok_or_else(|| anyhow!("No password found in {}", path.display()))?,
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn build_client() -> Result<Client> {
    Ok(Client::builder()
        .redirect(Policy::none())
        .cookie_store(true)
        .user_agent("sisar-download/1.0")
        .build()?)
}

/// Pre-signed S3 URLs carry auth in query params; adding an `Authorization`
/// header causes HTTP 400 ("only one auth mechanism allowed").
fn is_presigned_s3(url: &Url) -> bool {
    url.query_pairs().any(|(k, _)| k == "X-Amz-Signature")
}

fn make_request(client: &Client, url: &Url, creds: &Creds) -> reqwest::RequestBuilder {
    if is_presigned_s3(url) {
        client.get(url.clone())
    } else {
        client
            .get(url.clone())
            .basic_auth(&creds.user, Some(&creds.pass))
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Download a NASA Earthdata file to `save_path`.
///
/// Handles EDL OAuth redirects, pre-signed S3 URLs, and 202 polling.
/// Credentials are read from `EARTHDATA_USER`/`EARTHDATA_PASS` env vars
/// or `~/.netrc`.
pub async fn download(url: &str, save_path: &Path) -> Result<()> {
    let creds = load_creds().context(
        "Could not load credentials. \
         Set EARTHDATA_USER/EARTHDATA_PASS or add a ~/.netrc entry for urs.earthdata.nasa.gov",
    )?;
    let client = build_client()?;
    let mut current_url = Url::parse(url).context("Invalid URL")?;

    let response = 'outer: loop {
        let resp = make_request(&client, &current_url, &creds)
            .send()
            .await
            .context("Request failed")?;

        match resp.status() {
            // Follow redirects manually to attach auth on every hop
            s if s.is_redirection() => {
                let loc = resp
                    .headers()
                    .get(LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| anyhow!("Redirect with no Location header"))?;
                current_url = current_url.join(loc).context("Bad redirect URL")?;
            }

            StatusCode::OK => break resp,

            // 202: file is being generated server-side, poll until ready
            StatusCode::ACCEPTED => {
                for attempt in 1..=MAX_POLLS {
                    let wait = Duration::from_secs(POLL_BASE_SECS * attempt as u64);
                    tokio::time::sleep(wait).await;

                    let poll = make_request(&client, &current_url, &creds)
                        .send()
                        .await
                        .context("Poll request failed")?;

                    match poll.status() {
                        StatusCode::OK => break 'outer poll,
                        StatusCode::ACCEPTED => continue,
                        s if s.is_redirection() => {
                            let loc = poll
                                .headers()
                                .get(LOCATION)
                                .and_then(|v| v.to_str().ok())
                                .ok_or_else(|| anyhow!("Redirect with no Location header"))?;
                            current_url = current_url.join(loc).context("Bad redirect URL")?;
                            continue 'outer;
                        }
                        other => return Err(anyhow!("Unexpected status {other} while polling")),
                    }
                }
                return Err(anyhow!("File not ready after {MAX_POLLS} polls"));
            }

            other => return Err(anyhow!("HTTP {other} from {current_url}")),
        }
    };

    if let Some(parent) = save_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Cannot create directory {}", parent.display()))?;
    }

    let mut file = fs::File::create(save_path)
        .with_context(|| format!("Cannot create {}", save_path.display()))?;

    let mut stream = response;
    while let Some(chunk) = stream.chunk().await? {
        file.write_all(&chunk)?;
    }

    Ok(())
}

/// Derive a filename from the last path segment of a URL.
pub fn filename_from_url(url: &str) -> Option<PathBuf> {
    Url::parse(url).ok().and_then(|u| {
        u.path_segments()
            .map(|s| s.collect::<Vec<_>>().join("/"))
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    })
}
