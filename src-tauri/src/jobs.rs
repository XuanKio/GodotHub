use serde::Serialize;
use std::{
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Engine,
    Templates,
}

#[derive(Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Queued,
    Downloading,
    Verifying,
    Extracting,
    Installing,
    Completed,
    Failed,
}

impl Stage {
    fn finished(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub label: String,
    pub kind: Kind,
    pub stage: Stage,
    pub downloaded: u64,
    pub total: u64,
    pub error: Option<String>,
    pub started_at: u64,
}

#[derive(Default)]
pub struct Jobs {
    entries: Mutex<Vec<(String, Job)>>,
    pub work_slot: tokio::sync::Mutex<()>,
}

impl Jobs {
    pub fn has_active(&self) -> bool {
        self.entries
            .lock()
            .expect("Job lock poisoned")
            .iter()
            .any(|(_, job)| !job.stage.finished())
    }

    pub fn all(&self) -> Vec<Job> {
        self.entries
            .lock()
            .expect("Job lock poisoned")
            .iter()
            .map(|(_, job)| job.clone())
            .collect()
    }

    pub fn reserve(
        &self,
        key: String,
        label: String,
        kind: Kind,
        total: u64,
    ) -> Result<Job, String> {
        let mut entries = self.entries.lock().expect("Job lock poisoned");
        if entries
            .iter()
            .any(|(active, job)| active == &key && !job.stage.finished())
        {
            return Err("An installation for this build is already in the queue.".into());
        }
        while entries.len() >= 100 {
            let Some(index) = entries.iter().rposition(|(_, job)| job.stage.finished()) else {
                return Err("The download queue is full. Wait for a download to finish.".into());
            };
            entries.remove(index);
        }
        let job = Job {
            id: Uuid::new_v4().to_string(),
            label,
            kind,
            stage: Stage::Queued,
            downloaded: 0,
            total,
            error: None,
            started_at: unix_millis(),
        };
        entries.insert(0, (key, job.clone()));
        Ok(job)
    }

    pub fn is_active(&self, key: &str) -> bool {
        self.entries
            .lock()
            .expect("Job lock poisoned")
            .iter()
            .any(|(active, job)| active == key && !job.stage.finished())
    }

    pub fn update(&self, app: &AppHandle, id: &str, change: impl FnOnce(&mut Job)) {
        let mut entries = self.entries.lock().expect("Job lock poisoned");
        if let Some((_, job)) = entries.iter_mut().find(|(_, job)| job.id == id) {
            change(job);
            let _ = app.emit("hub://job", &*job);
        }
    }
}

pub fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
