use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{state::PeerInfo, types::ByteString};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistState {
    pub path: PathBuf,
    pub peer_id: ByteString,
    pub dht_peers: Vec<PeerInfo>,
}

impl PersistState {
    pub fn load(path: &Path) -> Result<Self> {
        let json = fs::read_to_string(path)?;
        serde_json::from_str(&json).context("deserialize error")
    }

    pub fn save(&self) -> Result<()> {
        fs::create_dir_all(self.path.parent().context("no parent")?)?;
        let json = serde_json::to_string(&self).context("serialize error")?;
        fs::write(&self.path, json)?;
        debug!("persist state written: {:?}", self);
        Ok(())
    }
}

impl Drop for PersistState {
    fn drop(&mut self) {
        if let Err(e) = self.save() {
            error!("{:#}", e.context("drop error"));
        }
    }
}
