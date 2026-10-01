use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const STATE_FILE_NAME: &str = "state.json";
const STATE_VERSION: u32 = 1;
const SECRET_LENGTH: usize = 32;
const MAX_STATE_FILE_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentState {
    pub version: u32,
    pub agent_id: Uuid,
    pub agent_secret: String,
}

impl AgentState {
    fn generate() -> Self {
        let mut secret = [0_u8; SECRET_LENGTH];
        rand::rng().fill_bytes(&mut secret);
        Self {
            version: STATE_VERSION,
            agent_id: Uuid::new_v4(),
            agent_secret: URL_SAFE_NO_PAD.encode(secret),
        }
    }

    fn validate(&self) -> Result<()> {
        if self.version != STATE_VERSION {
            bail!("unsupported state file version {}", self.version)
        }
        let secret = URL_SAFE_NO_PAD
            .decode(&self.agent_secret)
            .context("state file contains an invalid agent secret")?;
        if secret.len() != SECRET_LENGTH {
            bail!("state file contains an invalid agent secret length")
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct StateStore {
    directory: PathBuf,
    path: PathBuf,
}

pub struct LoadedState {
    pub state: AgentState,
    pub newly_created: bool,
}

impl StateStore {
    pub fn open(explicit_directory: Option<&Path>) -> Result<Self> {
        if let Some(directory) = explicit_directory {
            return Self::prepare(directory.to_owned()).with_context(|| {
                format!(
                    "failed to prepare requested state directory {}",
                    directory.display()
                )
            });
        }

        let preferred = default_state_directory()?;
        match Self::prepare(preferred.clone()) {
            Ok(store) => Ok(store),
            Err(primary_error) => {
                let fallback = std::env::current_dir()
                    .context("failed to determine the current directory")?
                    .join("data");
                if preferred == fallback {
                    return Err(primary_error).with_context(|| {
                        format!("failed to prepare state directory {}", preferred.display())
                    });
                }
                tracing::warn!(
                    preferred = %preferred.display(),
                    fallback = %fallback.display(),
                    reason = %primary_error,
                    "default state directory is not writable; using local data directory"
                );
                Self::prepare(fallback).context("failed to prepare fallback state directory")
            }
        }
    }

    fn prepare(directory: PathBuf) -> Result<Self> {
        fs::create_dir_all(&directory)
            .with_context(|| format!("failed to create {}", directory.display()))?;
        secure_directory(&directory)?;
        verify_directory_is_writable(&directory)?;
        let path = directory.join(STATE_FILE_NAME);
        Ok(Self { directory, path })
    }

    pub fn load_or_create(&self) -> Result<LoadedState> {
        match self.load() {
            Ok(Some(state)) => Ok(LoadedState {
                state,
                newly_created: false,
            }),
            Ok(None) => Ok(LoadedState {
                state: self.create()?,
                newly_created: true,
            }),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    fn load(&self) -> Result<Option<AgentState>> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to inspect {}", self.path.display()));
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!("state path {} is not a regular file", self.path.display())
        }
        if metadata.len() > MAX_STATE_FILE_BYTES {
            bail!("state file is unexpectedly large")
        }
        secure_state_file(&self.path)?;

        let file = File::open(&self.path)
            .with_context(|| format!("failed to open {}", self.path.display()))?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_STATE_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .context("failed to read state file")?;
        if bytes.len() as u64 > MAX_STATE_FILE_BYTES {
            bail!("state file is unexpectedly large")
        }
        let state: AgentState =
            serde_json::from_slice(&bytes).context("failed to parse state file")?;
        state.validate()?;
        Ok(Some(state))
    }

    fn create(&self) -> Result<AgentState> {
        let state = AgentState::generate();
        let mut serialized = serde_json::to_vec_pretty(&state)
            .context("failed to serialize newly generated agent state")?;
        serialized.push(b'\n');

        let temporary_path = self
            .directory
            .join(format!(".{STATE_FILE_NAME}.{}.tmp", Uuid::new_v4()));
        let creation_result = (|| -> Result<()> {
            let mut file = create_private_file(&temporary_path)?;
            file.write_all(&serialized)
                .context("failed to write temporary state file")?;
            file.sync_all()
                .context("failed to flush temporary state file")?;
            drop(file);
            fs::rename(&temporary_path, &self.path)
                .with_context(|| format!("failed to install {}", self.path.display()))?;
            secure_state_file(&self.path)?;
            Ok(())
        })();

        if let Err(error) = creation_result {
            let _ = fs::remove_file(&temporary_path);
            if self.path.exists() {
                return self
                    .load()?
                    .context("state file appeared during creation but could not be loaded");
            }
            return Err(error);
        }

        Ok(state)
    }
}

fn verify_directory_is_writable(directory: &Path) -> Result<()> {
    let probe_path = directory.join(format!(".write-test-{}", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let file = create_private_file(&probe_path)?;
        file.sync_all()
            .context("failed to flush state write probe")?;
        drop(file);
        fs::remove_file(&probe_path).context("failed to remove state write probe")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&probe_path);
    }
    result
}

fn create_private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    options
        .open(path)
        .with_context(|| format!("failed to create private file {}", path.display()))
}

#[cfg(unix)]
fn secure_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to restrict permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn secure_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn secure_state_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to restrict permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn secure_state_file(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(target_os = "windows")]
fn default_state_directory() -> Result<PathBuf> {
    Ok(std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("PROGRAMDATA"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("PingLake"))
}

#[cfg(target_os = "linux")]
fn default_state_directory() -> Result<PathBuf> {
    Ok(PathBuf::from("/var/lib/pinglake"))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn default_state_directory() -> Result<PathBuf> {
    Ok(std::env::current_dir()
        .context("failed to determine the current directory")?
        .join("data"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trip_preserves_identity() {
        let directory = tempfile::tempdir().unwrap();
        let store = StateStore::open(Some(directory.path())).unwrap();
        let created = store.load_or_create().unwrap();
        let loaded = store.load_or_create().unwrap();

        assert!(created.newly_created);
        assert!(!loaded.newly_created);
        assert_eq!(created.state.agent_id, loaded.state.agent_id);
        assert_eq!(created.state.agent_secret, loaded.state.agent_secret);
        assert_eq!(
            URL_SAFE_NO_PAD
                .decode(&loaded.state.agent_secret)
                .unwrap()
                .len(),
            SECRET_LENGTH
        );
    }

    #[test]
    fn serialized_state_rejects_unknown_fields() {
        let state = AgentState::generate();
        let mut value = serde_json::to_value(state).unwrap();
        value["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<AgentState>(value).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn state_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let store = StateStore::open(Some(directory.path())).unwrap();
        store.load_or_create().unwrap();
        let mode = fs::metadata(directory.path().join(STATE_FILE_NAME))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
