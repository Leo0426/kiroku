use kiroku_core::{
    localization::{self, NfoLanguage},
    Metadata,
};
#[test]
fn translation_retains_original_and_lock_wins() {
    let mut m = Metadata::default();
    m.code = "SMPL-001".into();
    m.fields.title = Some("日本語".into());
    m.fields.plot = Some("原文简介".into());
    localization::capture_original(&mut m); // Fresh source fields, before AI.
    kiroku_core::pipeline::apply_translation(
        &mut m,
        kiroku_core::translate::Translated {
            title: Some("中文".into()),
            plot: Some("中文简介".into()),
        },
    );
    assert_eq!(m.text_versions.original.plot.as_deref(), Some("原文简介"));
    let p = localization::project(&m, &NfoLanguage::Original);
    assert_eq!(p.metadata.fields.title.as_deref(), Some("日本語"));
    m.locked_fields.insert("title".into());
    m.fields.title = Some("".into());
    assert_eq!(
        localization::project(&m, &NfoLanguage::Original)
            .metadata
            .fields
            .title
            .as_deref(),
        Some("")
    );
}
#[test]
fn old_sidecar_does_not_guess_language() {
    let mut m = Metadata::default();
    m.code = "SMPL-001".into();
    m.fields.title = Some("中文".into());
    m.fields.plot = Some("现在".into());
    let p = localization::project(&m, &NfoLanguage::Original);
    assert_eq!(p.metadata.fields.plot, m.fields.plot);
    assert_eq!(p.warnings.len(), 2);
    let xml = kiroku_core::nfo::to_nfo(&m);
    assert!(!xml.contains("<originaltitle>"));
}
#[test]
fn partial_refresh_keeps_other_original_and_invalidates_affected_translation() {
    let mut before = Metadata::default();
    before.code = "SMPL-001".into();
    before.fields.title = Some("old title".into());
    before.fields.plot = Some("old plot".into());
    localization::capture_original(&mut before); // Fresh source fields, before AI.
    kiroku_core::pipeline::apply_translation(
        &mut before,
        kiroku_core::translate::Translated {
            title: Some("旧题".into()),
            plot: Some("旧简介".into()),
        },
    );
    let mut candidate = Metadata::default();
    candidate.code = before.code.clone();
    candidate.fields.title = Some("new title".into());
    candidate.fields.plot = Some("new plot".into());
    let after =
        kiroku_core::refresh::merge_selected(&before, &candidate, &["title".into()], true).unwrap();
    assert_eq!(
        after.text_versions.original.title.as_deref(),
        Some("new title")
    );
    assert_eq!(
        after.text_versions.original.plot.as_deref(),
        Some("old plot")
    );
    assert!(after.text_versions.localized["zh-Hans"].title.is_none());
    assert_eq!(
        after.text_versions.localized["zh-Hans"].plot.as_deref(),
        Some("旧简介")
    );
}
#[test]
fn explicit_export_preserves_unknown_xml_and_undo() {
    use std::fs;
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("SMPL-001");
    fs::create_dir(&dir).unwrap();
    let mut m = Metadata::default();
    m.code = "SMPL-001".into();
    m.fields.title = Some("日本語".into());
    m.fields.plot = Some("原文".into());
    localization::capture_original(&mut m); // Fresh source fields, before AI.
    kiroku_core::pipeline::apply_translation(
        &mut m,
        kiroku_core::translate::Translated {
            title: Some("中文".into()),
            plot: Some("译文".into()),
        },
    );
    let xml = kiroku_core::nfo::to_nfo(&m).replace(
        "</movie>",
        "<playcount>7</playcount><custom attr=\"yes\">kept</custom></movie>",
    );
    fs::write(dir.join("movie.nfo"), &xml).unwrap();
    fs::write(dir.join("kiroku.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    fs::write(dir.join("SMPL-001.mp4"), b"video").unwrap();
    let p =
        localization::export_preview(root.path(), &dir, &m.code, NfoLanguage::Original).unwrap();
    assert!(p.changes.iter().any(|c| c.field == "title"));
    assert_eq!(p.after.fields.title, m.fields.title);
    let out = kiroku_core::revision::apply(&p).unwrap();
    assert!(out.error.is_none());
    let exported = fs::read_to_string(dir.join("movie.nfo")).unwrap();
    assert!(exported.contains("<title>SMPL-001 日本語</title>"));
    assert!(exported.contains("<playcount>7</playcount><custom attr=\"yes\">kept</custom>"));
    let mut journal = out.journal;
    let failures = journal.rollback();
    assert!(failures.is_empty());
    assert_eq!(fs::read_to_string(dir.join("movie.nfo")).unwrap(), xml);
    assert_eq!(fs::read(dir.join("SMPL-001.mp4")).unwrap(), b"video");
}
#[test]
fn language_export_removes_duplicate_titles_and_keeps_playcount() {
    let mut before = Metadata::default();
    before.code = "SMPL-001".into();
    before.fields.title = Some("译文".into());
    before.fields.plot = Some("译简介".into());
    before.text_versions.original.title = Some("原題".into());
    before.text_versions.original.plot = Some("原文".into());
    let mut after = before.clone();
    after.nfo_language = NfoLanguage::Original;
    let xml="<movie><title lang=\"a\">one</title><title lang=\"b\">two</title><plot>one</plot><plot>two</plot><playcount>9</playcount></movie>";
    let output = kiroku_core::nfo::merge_revision(xml, &before, &after).unwrap();
    assert_eq!(output.matches("<title").count(), 1);
    assert_eq!(output.matches("<plot").count(), 1);
    assert!(output.contains("<originaltitle>SMPL-001 原題</originaltitle>"));
    assert!(output.contains("<playcount>9</playcount>"));
}
#[test]
fn supplied_language_text_is_preserved_selected_and_locked_empty_wins() {
    let mut m = Metadata::default();
    m.code = "SMPL-001".into();
    m.fields.title = Some(String::new());
    m.locked_fields.insert("title".into());
    let after = localization::set_localized(
        &m,
        "EN-us",
        localization::TextVersion {
            title: Some("English".into()),
            plot: Some("Plot".into()),
            ..Default::default()
        },
        true,
    )
    .unwrap();
    assert_eq!(after.fields.title, Some(String::new()));
    assert_eq!(after.fields.plot.as_deref(), Some("Plot"));
    assert!(after.text_versions.localized.contains_key("en-US"));
    assert!(after.text_versions.original.plot.is_none());
    assert!(localization::set_localized(&m, "../../", Default::default(), false).is_err());
}
#[test]
fn source_reselect_invalidates_only_adopted_field() {
    use kiroku_core::{metadata::SourceSnapshot, PartialMetadata};
    let mut before = Metadata::default();
    before.code = "SMPL-001".into();
    before.fields.title = Some("原題".into());
    before.fields.plot = Some("旧原文".into());
    localization::capture_original(&mut before); // Fresh source fields, before AI.
    kiroku_core::pipeline::apply_translation(
        &mut before,
        kiroku_core::translate::Translated {
            title: Some("旧标题".into()),
            plot: Some("旧译文".into()),
        },
    );
    let code = kiroku_core::code::parse_code(&before.code).unwrap();
    before.raw_sources.insert(
        "replacement".into(),
        SourceSnapshot::new(
            &code,
            "https://example.test",
            &PartialMetadata {
                title: Some("新原題".into()),
                plot: Some("新原文".into()),
                ..Default::default()
            },
        ),
    );
    let after = kiroku_core::refresh::select_source(&before, "title", "replacement").unwrap();
    assert_eq!(
        after.text_versions.original.title.as_deref(),
        Some("新原題")
    );
    assert!(after.text_versions.localized["zh-Hans"].title.is_none());
    assert_eq!(after.text_versions.original.plot.as_deref(), Some("旧原文"));
    assert_eq!(
        after.text_versions.localized["zh-Hans"].plot.as_deref(),
        Some("旧译文")
    );
    assert!(after.locked_fields.contains("title"));
}
#[test]
fn old_serialized_sidecar_gets_defaults_without_language_guessing() {
    let mut before = Metadata::default();
    before.code = "SMPL-001".into();
    before.fields.title = Some("中文".into());
    before.fields.plot = Some("中文简介".into());
    let mut json = serde_json::to_value(&before).unwrap();
    json.as_object_mut().unwrap().remove("text_versions");
    json.as_object_mut().unwrap().remove("nfo_language");
    let loaded: Metadata = serde_json::from_value(json).unwrap();
    assert_eq!(loaded.nfo_language, NfoLanguage::Current);
    assert!(loaded.text_versions.original.plot.is_none());
    assert_eq!(
        localization::project(&loaded, &NfoLanguage::Original)
            .warnings
            .len(),
        2
    );
}
#[test]
fn export_preview_rejects_replaced_movie_directory_even_with_identical_bytes() {
    use std::fs;
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("SMPL-001");
    fs::create_dir(&dir).unwrap();
    let mut m = Metadata::default();
    m.code = "SMPL-001".into();
    m.fields.title = Some("Current".into());
    let side = serde_json::to_vec(&m).unwrap();
    let nfo = kiroku_core::nfo::to_nfo(&m);
    fs::write(dir.join("kiroku.json"), &side).unwrap();
    fs::write(dir.join("movie.nfo"), &nfo).unwrap();
    let p =
        localization::export_preview(root.path(), &dir, &m.code, NfoLanguage::Original).unwrap();
    fs::rename(&dir, root.path().join("old")).unwrap();
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("kiroku.json"), &side).unwrap();
    fs::write(dir.join("movie.nfo"), &nfo).unwrap();
    assert!(kiroku_core::revision::apply(&p).is_err());
}
#[test]
fn captured_original_title_keeps_its_actual_source_provenance() {
    let mut m = Metadata::default();
    m.fields.title = Some("译标题".into());
    m.fields.original_title = Some("原題".into());
    m.provenance
        .insert("title".into(), "localized-source".into());
    m.provenance
        .insert("original_title".into(), "original-source".into());
    localization::capture_original(&mut m);
    assert_eq!(
        m.text_versions
            .original
            .provenance
            .get("title")
            .map(String::as_str),
        Some("original-source")
    );
}
#[test]
fn explicit_empty_localized_title_exports_code_and_preserves_real_originaltitle() {
    let mut m=Metadata::default();m.code="SMPL-001".into();m.fields.title=Some("Current".into());m.fields.original_title=Some("原題".into());m.nfo_language=NfoLanguage::ZhHans;m.text_versions.localized.insert("zh-Hans".into(),localization::TextVersion{title:Some(String::new()),..Default::default()});let xml=kiroku_core::nfo::to_nfo(&m);assert!(xml.contains("<title>SMPL-001</title>"));assert!(xml.contains("<originaltitle>SMPL-001 原題</originaltitle>"));
}
