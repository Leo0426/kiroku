//! Pure batch edit semantics; persistence is handled by revision previews.
use crate::Metadata;
use serde::Deserialize;
use std::io;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BatchEdit {
    Favorite {
        favorite: bool,
    },
    Tags {
        add: Vec<String>,
        remove: Vec<String>,
    },
}
impl BatchEdit {
    pub fn apply_to(&self, before: &Metadata) -> io::Result<Metadata> {
        let mut after = before.clone();
        match self {
            Self::Favorite { favorite } => after.favorite = *favorite,
            Self::Tags { add, remove } => {
                let clean = |values: &[String]| -> io::Result<Vec<String>> {
                    if values.len() > 50 {
                        return Err(io::Error::other("一次最多修改 50 个标签"));
                    }
                    let mut out = vec![];
                    for value in values {
                        let value = value.trim();
                        if value.is_empty()
                            || value.len() > 200
                            || value.chars().any(char::is_control)
                        {
                            return Err(io::Error::other("标签不能为空、超过 200 字节或含控制字符"));
                        }
                        if !out.iter().any(|s| s == value) {
                            out.push(value.to_owned())
                        }
                    }
                    Ok(out)
                };
                let add = clean(add)?;
                let remove = clean(remove)?;
                if add.is_empty() && remove.is_empty() {
                    return Err(io::Error::other("请填写要添加或移除的标签"));
                }
                if add.iter().any(|v| remove.contains(v)) {
                    return Err(io::Error::other("同一标签不能同时添加与移除"));
                }
                after.fields.tags.retain(|v| !remove.contains(v));
                for value in add {
                    if !after.fields.tags.contains(&value) {
                        after.fields.tags.push(value)
                    }
                }
                if after.fields.tags != before.fields.tags {
                    after.locked_fields.insert("tags".into());
                    after.provenance.insert("tags".into(), "manual".into());
                    after.name_projection.remove("tags");
                }
            }
        }
        Ok(after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tags_preserve_unrelated_values_lock_changes_and_skip_noops() {
        let mut before = Metadata::default();
        before.fields.tags = vec!["保留".into(), "旧标签".into()];
        let edit = BatchEdit::Tags {
            add: vec![" 新标签 ".into(), "新标签".into()],
            remove: vec!["旧标签".into()],
        };
        let after = edit.apply_to(&before).unwrap();
        assert_eq!(after.fields.tags, vec!["保留", "新标签"]);
        assert!(after.locked_fields.contains("tags"));
        assert_eq!(after.provenance["tags"], "manual");
        assert_eq!(edit.apply_to(&after).unwrap(), after);
        let unchanged = BatchEdit::Tags {
            add: vec!["保留".into()],
            remove: vec![],
        }
        .apply_to(&before)
        .unwrap();
        assert_eq!(unchanged, before);
        let favorite = BatchEdit::Favorite { favorite: true }
            .apply_to(&before)
            .unwrap();
        assert!(favorite.favorite);
        assert_eq!(favorite.fields, before.fields);
        assert_eq!(favorite.locked_fields, before.locked_fields);
    }
    #[test]
    fn conflicting_and_unbounded_tags_are_rejected() {
        for edit in [
            BatchEdit::Tags {
                add: vec!["same".into()],
                remove: vec!["same".into()],
            },
            BatchEdit::Tags {
                add: vec!["x".repeat(201)],
                remove: vec![],
            },
            BatchEdit::Tags {
                add: vec!["\n".into()],
                remove: vec![],
            },
        ] {
            assert!(edit.apply_to(&Metadata::default()).is_err())
        }
    }
}
