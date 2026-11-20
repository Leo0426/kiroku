//! Overrides for one import preview. Paths stay relative to the managed library.
use crate::{placement::OrganizeMode, settings::Settings, template};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRules {
    pub mode: OrganizeMode,
    pub subdirectory: String,
    pub directory_template: String,
    pub file_template: String,
    pub plex_naming: bool,
}

pub(crate) fn validate_literal(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 160
        || value.starts_with('.')
        || value.ends_with('.')
        || value != value.trim()
        || value
            .chars()
            .any(|c| c.is_control() || r#"/\:*?"<>|{}[]"#.contains(c))
        || crate::organize::sanitize(value) != value
    {
        return Err("名称需为普通文件名，不能包含路径分隔符、占位符或保留字符".into());
    }
    Ok(())
}

impl ImportRules {
    pub fn project(&self, default: &Settings) -> Result<Settings, String> {
        if self.directory_template.trim().is_empty() {
            return Err("请填写目录模板".into());
        }
        if self.directory_template != default.dir_template {
            template::validate_template(&self.directory_template, false)?;
        }
        if self.file_template != default.file_template {
            template::validate_file_template(&self.file_template)?;
        }
        let mut effective = default.clone();
        effective.dir_template = if self.subdirectory.is_empty() {
            self.directory_template.clone()
        } else {
            for part in self.subdirectory.split('/') {
                validate_literal(part)?;
            }
            format!("{}/{}", self.subdirectory, self.directory_template)
        };
        effective.file_template = self.file_template.clone();
        effective.plex.naming = self.plex_naming;
        effective.organize_mode = self.mode;
        Ok(effective)
    }
}
