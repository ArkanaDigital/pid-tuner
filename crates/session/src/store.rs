//! On-disk layout:
//! `<root>/<session-id>/session.json`, `logs/`, `analysis/`, `snapshots/`, `report/`.
//! Writes are atomic (temp file + rename).

use crate::model::Session;
use crate::{Result, SessionError};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct SessionStore {
    root: PathBuf,
}

impl SessionStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn dir(&self, id: Uuid) -> PathBuf {
        self.root.join(id.to_string())
    }

    pub fn ensure_dirs(&self, id: Uuid) -> Result<PathBuf> {
        let d = self.dir(id);
        for sub in ["logs", "analysis", "snapshots", "report"] {
            fs::create_dir_all(d.join(sub))?;
        }
        Ok(d)
    }

    pub fn save(&self, s: &Session) -> Result<()> {
        let d = self.ensure_dirs(s.id)?;
        let tmp = d.join("session.json.tmp");
        let bytes = serde_json::to_vec_pretty(s)?;
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, d.join("session.json"))?;
        Ok(())
    }

    pub fn load(&self, id: Uuid) -> Result<Session> {
        let p = self.dir(id).join("session.json");
        if !p.exists() {
            return Err(SessionError::NotFound(id.to_string()));
        }
        Ok(serde_json::from_slice(&fs::read(p)?)?)
    }

    pub fn list(&self) -> Result<Vec<Session>> {
        let mut out = Vec::new();
        if !self.root.exists() {
            return Ok(out);
        }
        for e in fs::read_dir(&self.root)? {
            let e = e?;
            let p = e.path().join("session.json");
            if p.exists() {
                if let Ok(s) = serde_json::from_slice::<Session>(&fs::read(p)?) {
                    out.push(s);
                }
            }
        }
        out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(out)
    }

    pub fn delete(&self, id: Uuid) -> Result<()> {
        let d = self.dir(id);
        if d.exists() {
            fs::remove_dir_all(d)?;
        }
        Ok(())
    }

    /// Copy a log file into the session; returns the relative path.
    pub fn import_file(&self, id: Uuid, src: &Path, label: &str) -> Result<String> {
        let d = self.ensure_dirs(id)?;
        let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("bbl");
        let name = format!("{label}.{ext}");
        fs::copy(src, d.join("logs").join(&name))?;
        Ok(format!("logs/{name}"))
    }

    pub fn write_rel(&self, id: Uuid, rel: &str, bytes: &[u8]) -> Result<()> {
        let d = self.ensure_dirs(id)?;
        let p = d.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = p.with_extension("tmp");
        fs::write(&tmp, bytes)?;
        fs::rename(tmp, p)?;
        Ok(())
    }

    pub fn read_rel(&self, id: Uuid, rel: &str) -> Result<Vec<u8>> {
        Ok(fs::read(self.dir(id).join(rel))?)
    }

    pub fn abs(&self, id: Uuid, rel: &str) -> PathBuf {
        self.dir(id).join(rel)
    }
}
