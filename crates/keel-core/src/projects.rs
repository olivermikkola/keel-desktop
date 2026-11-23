use crate::documents::Document;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub instructions: String,
    pub workspace_path: Option<String>,
    #[serde(default)]
    pub documents: Vec<Document>,
}

pub fn valid_id(id: &str) -> Result<()> {
    if id.is_empty() || id.len() > 80 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        bail!("Invalid record identifier");
    }
    Ok(())
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("json.{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(temp, path)?;
    Ok(())
}

pub fn save_project(data: &Path, project: &Project) -> Result<()> {
    valid_id(&project.id)?;
    if project.name.trim().is_empty() || project.name.chars().count() > 80 {
        bail!("Give your project a name of up to 80 characters");
    }
    if project.instructions.chars().count() > 6000 || project.documents.len() > 12 {
        bail!("Use up to 6000 instruction characters and 12 reference documents per project");
    }
    write_json(
        &data.join("projects").join(format!("{}.json", project.id)),
        project,
    )
}

pub fn load_project(data: &Path, id: &str) -> Result<Project> {
    valid_id(id)?;
    Ok(serde_json::from_slice(&std::fs::read(
        data.join("projects").join(format!("{id}.json")),
    )?)?)
}

pub fn list_projects(data: &Path) -> Vec<Project> {
    let Ok(entries) = std::fs::read_dir(data.join("projects")) else {
        return vec![];
    };
    let mut result: Vec<Project> = entries
        .flatten()
        .filter_map(|e| {
            if e.path().extension()?.to_str()? != "json" {
                return None;
            }
            serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok()
        })
        .collect();
    result.sort_by_key(|p| p.name.to_lowercase());
    result
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectInfo {
    pub id: String,
    pub name: String,
    pub workspace_path: Option<String>,
    pub document_count: usize,
}
pub fn project_info(data: &Path) -> Vec<ProjectInfo> {
    list_projects(data)
        .into_iter()
        .map(|p| ProjectInfo {
            id: p.id,
            name: p.name,
            workspace_path: p.workspace_path,
            document_count: p.documents.len(),
        })
        .collect()
}
