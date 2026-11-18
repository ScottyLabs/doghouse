use crate::*;
use std::sync::{Arc, Mutex};

static CLIENT: Mutex<(u64, Option<Arc<Client>>)> = Mutex::new((0, None));
static INITIALIZE: Mutex<()> = Mutex::new(());

pub(crate) struct Client {
    pub(crate) api: collar::Api,
}

pub(crate) fn client() -> Result<Arc<Client>> {
    if let Some(client) = CLIENT
        .lock()
        .map_err(|_| anyhow::anyhow!("Forge client unavailable"))?
        .1
        .as_ref()
        .map(Arc::clone)
    {
        return Ok(client);
    }
    // Only one background reader resolves credentials; reset never takes this lock.
    let _initializing = INITIALIZE
        .lock()
        .map_err(|_| anyhow::anyhow!("Forge initialization unavailable"))?;
    let generation = {
        let slot = CLIENT
            .lock()
            .map_err(|_| anyhow::anyhow!("Forge client unavailable"))?;
        if let Some(client) = slot.1.as_ref() {
            return Ok(Arc::clone(client));
        }
        slot.0
    };
    // Credential resolution can use the network; never hold the UI's cache lock during it.
    let config = collar::Config::load(None, false, false)?;
    let client = Arc::new(Client {
        api: collar::authenticated_api(collar::host_url("git.cmu.dev")?, &config)?,
    });
    let mut slot = CLIENT
        .lock()
        .map_err(|_| anyhow::anyhow!("Forge client unavailable"))?;
    if slot.0 == generation {
        return Ok(Arc::clone(slot.1.get_or_insert(client)));
    }
    Ok(client)
}

/// A manual refresh picks up credentials changed by `collar auth login`.
pub(crate) fn reset() -> Result<()> {
    let mut slot = CLIENT
        .lock()
        .map_err(|_| anyhow::anyhow!("Forge client unavailable"))?;
    slot.0 = slot.0.wrapping_add(1);
    slot.1 = None;
    Ok(())
}

impl Client {
    pub(crate) fn read(&self, path: &str, fields: &[(&str, &str)]) -> Result<Value> {
        let _permit = shared::read_permit()?;
        self.read_with_permit(path, fields)
    }
    /// Caller holds the shared foreground or background permit.
    pub(crate) fn read_with_permit(&self, path: &str, fields: &[(&str, &str)]) -> Result<Value> {
        self.api.json(
            collar::Method::GET,
            &path.split('/').collect::<Vec<_>>(),
            &fields
                .iter()
                .map(|(name, value)| ((*name).into(), (*value).into()))
                .collect::<Vec<_>>(),
            None,
        )
    }

    pub(crate) fn list(
        &self,
        path: &str,
        fields: &[(&str, &str)],
        maximum: usize,
    ) -> Result<Vec<Value>> {
        shared::collect_pages(
            |page| {
                let page = page.to_string();
                let mut query = fields.to_vec();
                query.extend([("page", page.as_str()), ("limit", "50")]);
                self.read(path, &query)?
                    .as_array()
                    .cloned()
                    .context("Unexpected GCD listing")
            },
            maximum,
        )
    }

    pub(crate) fn write(&self, path: &str, method: collar::Method, body: &Value) -> Result<Value> {
        let response = self
            .api
            .text(
                method,
                &path.split('/').collect::<Vec<_>>(),
                &[],
                Some(body),
            )
            .context("GCD write failed; inspect before retrying")?;
        if response.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&response)
            .context("GCD write may have completed; refresh before retrying")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use tiny_http::{Header, Response, Server};

    #[test]
    fn failed_and_uncertain_writes_are_never_retried() {
        for (status, response) in [(500, "failure"), (200, "invalid JSON")] {
            let server = Server::http("127.0.0.1:0").unwrap();
            let directory = tempfile::tempdir().unwrap();
            let client = Client {
                api: collar::Api::new(
                    collar::host_url(&format!("http://{}", server.server_addr())).unwrap(),
                    Some("fixture-token".into()),
                )
                .unwrap()
                .with_read_cache(collar::ReadCache {
                    directory: Some(directory.path().into()),
                    refresh: false,
                }),
            };
            let worker = std::thread::spawn(move || {
                let mut request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                assert_eq!(request.method().as_str(), "POST");
                let body: Value = serde_json::from_reader(request.as_reader()).unwrap();
                assert_eq!(body, json!({"body":"comment"}));
                request
                    .respond(Response::from_string(response).with_status_code(status))
                    .unwrap();
                assert!(
                    server
                        .recv_timeout(Duration::from_millis(50))
                        .unwrap()
                        .is_none()
                );
            });
            assert!(
                client
                    .write(
                        "repos/ScottyLabs/example/issues/7/comments",
                        collar::Method::POST,
                        &json!({"body":"comment"})
                    )
                    .unwrap_err()
                    .to_string()
                    .contains("before retrying")
            );
            worker.join().unwrap();
        }
    }

    #[test]
    #[ignore = "explicit local benchmark; requires SHEPHERD_BENCH_COLLAR"]
    fn benchmark_reused_client_against_collar_subprocesses() {
        const REQUESTS: usize = 25;
        let executable = std::env::var_os("SHEPHERD_BENCH_COLLAR")
            .expect("select the baseline Collar executable");
        let directory = tempfile::tempdir().unwrap();
        let server = Server::http("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", server.server_addr());
        let worker = std::thread::spawn(move || {
            for _ in 0..REQUESTS * 6 {
                let request = server
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap()
                    .unwrap();
                assert_eq!(request.url(), "/api/v1/user");
                request
                    .respond(
                        Response::from_string(r#"{"login":"fixture"}"#)
                            .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap()),
                    )
                    .unwrap();
            }
        });
        let mut subprocess = Vec::new();
        let mut library = Vec::new();
        for _ in 0..3 {
            let started = Instant::now();
            for _ in 0..REQUESTS {
                let output = Command::new(&executable)
                    .args(["--no-config", "--host", &origin, "--cache-dir"])
                    .arg(directory.path())
                    .args(["api", "user", "--method", "GET"])
                    .env("COLLAR_HOST", &origin)
                    .env("COLLAR_TOKEN", "fixture-token")
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(
                    serde_json::from_slice::<Value>(&output.stdout).unwrap()["login"],
                    "fixture"
                );
            }
            subprocess.push(started.elapsed().as_secs_f64());
            let started = Instant::now();
            let client = Client {
                api: collar::Api::new(
                    collar::host_url(&origin).unwrap(),
                    Some("fixture-token".into()),
                )
                .unwrap()
                .with_read_cache(collar::ReadCache {
                    directory: Some(directory.path().into()),
                    refresh: false,
                }),
            };
            for _ in 0..REQUESTS {
                assert_eq!(client.read("user", &[]).unwrap()["login"], "fixture");
            }
            library.push(started.elapsed().as_secs_f64());
        }
        worker.join().unwrap();
        println!(
            "{REQUESTS} loopback reads per batch; subprocess_seconds={subprocess:?}; library_seconds={library:?}"
        );
    }
}
