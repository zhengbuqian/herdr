use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::endpoint::{ClientEndpointId, SavedSshEndpoint};
use super::shell::ClientShellState;

const SCRIPT: &str = include_str!("../platform/unix_home_workspace.py");
const JOB_TIMEOUT: Duration = Duration::from_secs(40);

struct HomeJob {
    child: Child,
    boot_id: String,
    started: Instant,
}

pub(super) struct HomeWorkspaces {
    api_socket: PathBuf,
    jobs: HashMap<ClientEndpointId, HomeJob>,
    retry_after: HashMap<ClientEndpointId, (String, Instant)>,
}

impl HomeWorkspaces {
    pub(super) fn new(api_socket: PathBuf) -> Self {
        Self {
            api_socket,
            jobs: HashMap::new(),
            retry_after: HashMap::new(),
        }
    }

    pub(super) fn tick(
        &mut self,
        shell: &ClientShellState,
        profiles: &[SavedSshEndpoint],
        now: Instant,
    ) {
        let finished = self
            .jobs
            .iter_mut()
            .filter_map(|(endpoint, job)| {
                if now.saturating_duration_since(job.started) >= JOB_TIMEOUT {
                    let _ = job.child.kill();
                    return Some(endpoint.clone());
                }
                match job.child.try_wait() {
                    Ok(Some(_)) | Err(_) => Some(endpoint.clone()),
                    Ok(None) => None,
                }
            })
            .collect::<Vec<_>>();
        for endpoint in finished {
            let Some(job) = self.jobs.remove(&endpoint) else {
                continue;
            };
            let success = match job.child.wait_with_output() {
                Ok(output) if output.status.success() => true,
                Ok(output) => {
                    tracing::warn!(?endpoint, error = %String::from_utf8_lossy(&output.stderr), "could not maintain home workspace");
                    false
                }
                Err(error) => {
                    tracing::warn!(?endpoint, %error, "could not wait for home workspace task");
                    false
                }
            };
            let delay = if success {
                Duration::from_millis(500)
            } else {
                Duration::from_secs(30)
            };
            self.retry_after
                .insert(endpoint, (job.boot_id, now + delay));
        }
        for (endpoint, boot_id) in shell.missing_home_workspaces() {
            if self.jobs.contains_key(&endpoint)
                || self
                    .retry_after
                    .get(&endpoint)
                    .is_some_and(|(previous_boot, until)| previous_boot == &boot_id && now < *until)
            {
                continue;
            }
            match self.spawn(&endpoint, profiles) {
                Ok(child) => {
                    self.jobs.insert(
                        endpoint,
                        HomeJob {
                            child,
                            boot_id,
                            started: now,
                        },
                    );
                }
                Err(error) => {
                    tracing::warn!(?endpoint, %error, "could not start home workspace task");
                    self.retry_after
                        .insert(endpoint, (boot_id, now + Duration::from_secs(30)));
                }
            }
        }
    }

    fn spawn(
        &self,
        endpoint: &ClientEndpointId,
        profiles: &[SavedSshEndpoint],
    ) -> io::Result<Child> {
        let mut command = if let ClientEndpointId::Ssh(id) = endpoint {
            let profile = profiles
                .iter()
                .find(|profile| &profile.id == id && profile.enabled)
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "SSH machine no longer configured")
                })?;
            let mut command = Command::new("ssh");
            command
                .args(["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", "--"])
                .arg(&profile.target)
                .arg(format!(
                    "python3 - '{}'",
                    profile.session.replace('\'', "'\"'\"'")
                ))
                .stdin(Stdio::piped());
            command
        } else {
            let mut command = Command::new("python3");
            command
                .arg("-c")
                .arg(SCRIPT)
                .arg("default")
                .arg(&self.api_socket)
                .stdin(Stdio::null());
            command
        };
        command.stdout(Stdio::null()).stderr(Stdio::piped());
        let mut child = command.spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            if let Err(error) = stdin.write_all(SCRIPT.as_bytes()) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
        Ok(child)
    }
}

impl Drop for HomeWorkspaces {
    fn drop(&mut self) {
        for job in self.jobs.values_mut() {
            let _ = job.child.kill();
            let _ = job.child.wait();
        }
    }
}
