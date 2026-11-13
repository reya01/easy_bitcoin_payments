//! Optional in-process supervisor for the barkd wallet daemon
//! (`EBP_MANAGED_BARKD=true`).
//!
//! Why: with a separate barkd container a beginner must copy a token out of
//! a container, paste it into a `.env` file, restart, and run a command to
//! see the recovery words. Here EBP owns barkd as a child process in the same
//! container, so the wizard can do those steps itself:
//!
//! * start barkd on loopback only (`127.0.0.1`), never exposed;
//! * read its bearer token with `barkd secret show` (on every boot, so the
//!   token is never stored by EBP);
//! * for the one-time recovery-words ceremony, restart barkd with
//!   `--expose-mnemonic`, read the words once, and restart it WITHOUT the
//!   flag right away;
//! * restart it if it dies (never with the mnemonic flag).
//!
//! Security notes: the child gets a CLEARED environment (it must not inherit
//! `EBP_KEY_ENCRYPTION_KEY`, `EBP_SETUP_TOKEN` or any other EBP secret), its
//! output is discarded (it may print sensitive material), and the token is
//! returned to the caller only, never logged.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

static INSTANCE: OnceLock<Arc<ManagedBarkd>> = OnceLock::new();

/// The process-wide managed barkd, if managed mode is on.
pub fn get() -> Option<Arc<ManagedBarkd>> {
    INSTANCE.get().cloned()
}

/// Register the managed barkd for this process.
pub fn install(m: ManagedBarkd) -> Arc<ManagedBarkd> {
    let a = Arc::new(m);
    let _ = INSTANCE.set(Arc::clone(&a));
    a
}

pub struct ManagedBarkd {
    bin: String,
    datadir: PathBuf,
    port: u16,
    child: Mutex<Option<Child>>,
}

impl ManagedBarkd {
    /// barkd's data lives in a `barkd` folder beside the EBP database (so one
    /// volume holds everything). The binary is `barkd` on PATH, or
    /// `EBP_BARKD_BIN`.
    pub fn new(database_path: &str) -> Self {
        let datadir = Path::new(database_path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .join("barkd");
        Self {
            bin: std::env::var("EBP_BARKD_BIN").unwrap_or_else(|_| "barkd".to_string()),
            datadir,
            port: 3000,
            child: Mutex::new(None),
        }
    }

    /// Where barkd keeps its wallet files (for the encrypted backup).
    pub fn datadir(&self) -> &Path {
        &self.datadir
    }

    pub fn api_url(&self) -> String {
        format!("http://127.0.0.1:{}/api/v1", self.port)
    }

    fn base_command(&self) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env(
                "HOME",
                self.datadir.parent().unwrap_or_else(|| Path::new("/")),
            )
            .stdin(Stdio::null());
        cmd
    }

    /// The barkd start command: loopback only, and with barkd's built-in
    /// wallet web page turned off (`--no-ui`, barkd 0.8.0): EBP is the only
    /// interface, and a second page that can move money has no place here.
    fn start_command(&self, expose_mnemonic: bool) -> Command {
        let mut cmd = self.base_command();
        cmd.arg("--datadir")
            .arg(&self.datadir)
            .args(["--host", "127.0.0.1", "--port"])
            .arg(self.port.to_string())
            .arg("--no-ui");
        if expose_mnemonic {
            cmd.arg("--expose-mnemonic");
        }
        cmd
    }

    fn spawn(&self, expose_mnemonic: bool) -> std::io::Result<Child> {
        std::fs::create_dir_all(&self.datadir)?;
        self.start_command(expose_mnemonic)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    }

    fn restart_blocking(&self, expose_mnemonic: bool) -> Result<(), String> {
        let mut guard = self
            .child
            .lock()
            .map_err(|_| "barkd supervisor lock poisoned".to_string())?;
        if let Some(mut old) = guard.take() {
            let _ = old.kill();
            let _ = old.wait();
        }
        let child = self
            .spawn(expose_mnemonic)
            .map_err(|e| format!("could not start barkd: {e}"))?;
        *guard = Some(child);
        Ok(())
    }

    /// (Re)start barkd and wait until it answers. `expose_mnemonic` is for
    /// the one-time recovery-words ceremony only.
    pub async fn restart(self: &Arc<Self>, expose_mnemonic: bool) -> Result<(), String> {
        let me = Arc::clone(self);
        tokio::task::spawn_blocking(move || me.restart_blocking(expose_mnemonic))
            .await
            .map_err(|e| format!("barkd restart task failed: {e}"))??;
        self.wait_ready().await
    }

    async fn wait_ready(&self) -> Result<(), String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| e.to_string())?;
        let url = format!("http://127.0.0.1:{}/ping", self.port);
        for _ in 0..60 {
            if let Ok(r) = client.get(&url).send().await {
                if r.status().is_success() {
                    return Ok(());
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Err("barkd did not become ready in 30 seconds".to_string())
    }

    /// barkd's bearer token (`barkd secret show`). Never logged.
    pub async fn token(self: &Arc<Self>) -> Result<String, String> {
        let me = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let out = me
                .base_command()
                .arg("--datadir")
                .arg(&me.datadir)
                .args(["secret", "show"])
                .stderr(Stdio::null())
                .output()
                .map_err(|e| format!("could not run barkd: {e}"))?;
            let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if out.status.success() && !token.is_empty() {
                Ok(token)
            } else {
                Err("barkd has no token yet".to_string())
            }
        })
        .await
        .map_err(|e| format!("barkd token task failed: {e}"))?
    }

    /// Keep barkd alive: if it exits, start it again (never with the
    /// mnemonic flag).
    pub async fn supervise(self: Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;
            let dead = match self.child.lock() {
                Ok(mut g) => match g.as_mut() {
                    Some(c) => matches!(c.try_wait(), Ok(Some(_))),
                    None => true,
                },
                Err(_) => false,
            };
            if dead {
                tracing::warn!("barkd exited; restarting it");
                if let Err(err) = self.restart(false).await {
                    tracing::error!(error = %err, "could not restart barkd");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datadir_sits_beside_the_database() {
        assert_eq!(
            ManagedBarkd::new("/data/ebp.sqlite3").datadir,
            PathBuf::from("/data/barkd")
        );
        assert_eq!(
            ManagedBarkd::new("ebp.sqlite3").datadir,
            PathBuf::from("./barkd")
        );
    }

    #[test]
    fn barkd_starts_on_loopback_without_its_own_wallet_page() {
        let m = ManagedBarkd::new("/data/ebp.sqlite3");
        let args = |expose| -> Vec<String> {
            m.start_command(expose)
                .get_args()
                .map(|a| a.to_string_lossy().to_string())
                .collect()
        };
        let plain = args(false);
        assert!(plain.contains(&"--no-ui".to_string()), "{plain:?}");
        assert!(
            plain.windows(2).any(|w| w == ["--host", "127.0.0.1"]),
            "{plain:?}"
        );
        assert!(!plain.contains(&"--expose-mnemonic".to_string()));
        let ceremony = args(true);
        assert!(ceremony.contains(&"--no-ui".to_string()));
        assert!(ceremony.contains(&"--expose-mnemonic".to_string()));
    }

    #[test]
    fn child_environment_is_cleared() {
        let m = ManagedBarkd::new("/data/ebp.sqlite3");
        let cmd = m.base_command();
        let envs: Vec<_> = cmd
            .get_envs()
            .map(|(k, _)| k.to_string_lossy().to_string())
            .collect();
        assert!(envs.iter().all(|k| k == "PATH" || k == "HOME"), "{envs:?}");
    }
}
