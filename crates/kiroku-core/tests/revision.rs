use kiroku_core::{journal::Journal, revision, Metadata};
use std::fs;
#[test]
fn edit_and_undo_restores_exact_bytes_and_preserves_video() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let dir = root.join("ABC-123");
    fs::create_dir(&dir).unwrap();
    let mut m = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    m.fields.title = Some("old".into());
    let old = serde_json::to_vec(&m).unwrap();
    fs::write(dir.join("kiroku.json"), &old).unwrap();
    fs::write(
        dir.join("movie.nfo"),
        b"<movie custom=\"keep\"><title>old</title><!-- original bytes --></movie>",
    )
    .unwrap();
    fs::write(dir.join("ABC-123.mp4"), b"video").unwrap();
    let preview =
        revision::preview(root, &dir, "ABC-123", serde_json::json!({"title":"new"})).unwrap();
    let out = revision::apply(&preview).unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    let edited: Metadata =
        serde_json::from_slice(&fs::read(dir.join("kiroku.json")).unwrap()).unwrap();
    assert_eq!(edited.fields.title.as_deref(), Some("new"));
    assert!(edited.locked_fields.contains("title"));
    let mut j = Journal::load(&out.journal.path()).unwrap();
    assert!(j.rollback().is_empty());
    assert_eq!(fs::read(dir.join("kiroku.json")).unwrap(), old);
    assert_eq!(
        fs::read(dir.join("movie.nfo")).unwrap(),
        b"<movie custom=\"keep\"><title>old</title><!-- original bytes --></movie>"
    );
    assert_eq!(fs::read(dir.join("ABC-123.mp4")).unwrap(), b"video");
}

#[test]
fn every_interruption_recovers_independent_backups_and_missing_files() {
    use kiroku_core::journal::{ReplacementCheckpoint as C, ReplacementInput};
    for point in [
        C::Intent,
        C::Backup(0),
        C::Backup(1),
        C::Prepared,
        C::Replaced(0),
        C::Replaced(1),
    ] {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        let a = root.join("movie.nfo");
        let b = root.join("kiroku.json");
        fs::write(&a, b"old").unwrap();
        let mut j = Journal::new(&root);
        let expected = {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(b"old"))
        };
        assert!(j
            .replace_files_observed(
                &[
                    ReplacementInput {
                        path: a.clone(),
                        expected: Some(expected),
                        bytes: b"new".to_vec()
                    },
                    ReplacementInput {
                        path: b.clone(),
                        expected: None,
                        bytes: b"sidecar".to_vec()
                    }
                ],
                &mut |p| if p == point {
                    Err(std::io::Error::other("injected failure"))
                } else {
                    Ok(())
                }
            )
            .is_err());
        let mut loaded = Journal::load(&j.path()).unwrap();
        assert!(loaded.rollback().is_empty(), "{point:?}");
        assert_eq!(fs::read(a).unwrap(), b"old");
        assert!(!b.exists());
        assert!(loaded.rollback().is_empty());
    }
}
#[test]
fn stale_preview_and_external_changes_refuse_overwrite() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let dir = root.join("ABC-123");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("movie.nfo"),
        "<movie><num>ABC-123</num><title>old</title></movie>",
    )
    .unwrap();
    let p = revision::preview(root, &dir, "ABC-123", serde_json::json!({"title":"new"})).unwrap();
    fs::write(dir.join("movie.nfo"), b"external").unwrap();
    assert!(revision::apply(&p).is_err());
    assert!(Journal::list(root).is_empty());
    fs::write(
        dir.join("movie.nfo"),
        "<movie><num>ABC-123</num><title>old</title></movie>",
    )
    .unwrap();
    let p = revision::preview(root, &dir, "ABC-123", serde_json::json!({"title":"new"})).unwrap();
    let out = revision::apply(&p).unwrap();
    assert!(out.error.is_none());
    fs::write(dir.join("movie.nfo"), b"external in-place edit").unwrap();
    let mut j = Journal::load(&out.journal.path()).unwrap();
    assert!(!j.rollback().is_empty());
    assert_eq!(
        fs::read(dir.join("movie.nfo")).unwrap(),
        b"external in-place edit"
    );
    let backup = j.replacements[0].backup.as_ref().unwrap();
    assert_ne!(fs::read(backup).unwrap(), b"external in-place edit");
}
#[test]
fn unknown_sidecar_state_preserved_and_only_changed_fields_locked_including_empty() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("ABC-123");
    fs::create_dir(&dir).unwrap();
    let mut meta = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    meta.fields.title = Some("old".into());
    meta.fields.original_title = Some("original".into());
    meta.fields.plot = Some("plot".into());
    let mut raw = serde_json::to_value(meta).unwrap();
    raw["raw_results"] = serde_json::json!({"site":{"title":"raw"}});
    raw.as_object_mut().unwrap().remove("locked_fields");
    fs::write(dir.join("kiroku.json"), serde_json::to_vec(&raw).unwrap()).unwrap();
    let p = revision::preview(
        t.path(),
        &dir,
        "ABC-123",
        serde_json::json!({"title":null,"actors":[],"plot":"plot"}),
    )
    .unwrap();
    assert_eq!(
        p.after.locked_fields.iter().collect::<Vec<_>>(),
        vec!["title"]
    );
    assert_eq!(p.after.provenance["title"], "manual");
    assert!(revision::apply(&p).unwrap().error.is_none());
    let edited: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("kiroku.json")).unwrap()).unwrap();
    assert_eq!(edited["raw_results"], raw["raw_results"]);
    assert_eq!(edited["original_title"], "original");
}
#[test]
fn unreadable_sidecar_and_multiple_nfos_rejected() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("ABC-123");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("movie.nfo"),
        "<movie><num>ABC-123</num><title>old</title></movie>",
    )
    .unwrap();
    fs::write(dir.join("kiroku.json"), b"broken").unwrap();
    assert!(revision::preview(
        t.path(),
        &dir,
        "ABC-123",
        serde_json::json!({"title":"new"})
    )
    .is_err());
    fs::remove_file(dir.join("kiroku.json")).unwrap();
    fs::write(dir.join("other.nfo"), b"second").unwrap();
    assert!(revision::preview(
        t.path(),
        &dir,
        "ABC-123",
        serde_json::json!({"title":"new"})
    )
    .is_err());
}
#[test]
fn manual_empty_title_does_not_reappear_from_original_in_nfo() {
    let mut m = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    m.fields.original_title = Some("original".into());
    m.locked_fields.insert("title".into());
    let nfo = kiroku_core::nfo::to_nfo(&m);
    assert!(!nfo.contains("<title>ABC-123 original</title>"));
    assert!(nfo.contains("<originaltitle>ABC-123 original</originaltitle>"));
}
#[test]
fn backup_corruption_before_commit_stops_all_replacements() {
    use kiroku_core::journal::{ReplacementCheckpoint as C, ReplacementInput};
    use sha2::{Digest, Sha256};
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let path = root.join("movie.nfo");
    fs::write(&path, b"old").unwrap();
    let mut j = Journal::new(&root);
    let backup = root
        .join(".kiroku")
        .join(format!("revision-{}", j.id))
        .join("0.before");
    let result = j.replace_files_observed(
        &[ReplacementInput {
            path: path.clone(),
            expected: Some(format!("{:x}", Sha256::digest(b"old"))),
            bytes: b"new".to_vec(),
        }],
        &mut |p| {
            if p == C::Prepared {
                fs::write(&backup, b"corrupt")?;
            }
            Ok(())
        },
    );
    assert!(result.is_err());
    assert_eq!(fs::read(path).unwrap(), b"old");
}
#[test]
fn real_backup_and_journal_write_failures_leave_targets_untouched() {
    use kiroku_core::journal::{ReplacementCheckpoint as C, ReplacementInput};
    use sha2::{Digest, Sha256};
    for backup_failure in [true, false] {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        let path = root.join("movie.nfo");
        fs::write(&path, b"old").unwrap();
        let mut j = Journal::new(&root);
        let storage = root.join(".kiroku").join(format!("revision-{}", j.id));
        if !backup_failure {
            fs::create_dir_all(j.path().with_extension("json.tmp")).unwrap();
        }
        let result = j.replace_files_observed(
            &[ReplacementInput {
                path: path.clone(),
                expected: Some(format!("{:x}", Sha256::digest(b"old"))),
                bytes: b"new".to_vec(),
            }],
            &mut |p| {
                if p == C::Intent && backup_failure {
                    fs::create_dir_all(storage.join("0.before"))?;
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read(path).unwrap(), b"old");
    }
}
#[test]
fn second_target_failure_and_log_failure_after_first_replace_are_recoverable() {
    use kiroku_core::journal::{ReplacementCheckpoint as C, ReplacementInput};
    use sha2::{Digest, Sha256};
    for journal_failure in [true, false] {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        let a = root.join("movie.nfo");
        let b = root.join("kiroku.json");
        fs::write(&a, b"old-a").unwrap();
        fs::write(&b, b"old-b").unwrap();
        let mut j = Journal::new(&root);
        let log_path=j.path();let blocked=log_path.with_extension("blocked");
        let result = j.replace_files_observed(
            &[
                (a.clone(), b"old-a", b"new-a"),
                (b.clone(), b"old-b", b"new-b"),
            ]
            .into_iter()
            .map(|(path, old, new)| ReplacementInput {
                path,
                expected: Some(format!("{:x}", Sha256::digest(old))),
                bytes: new.to_vec(),
            })
            .collect::<Vec<_>>(),
            &mut |p| {
                if p == C::Replaced(0) {
                    if journal_failure {
                        fs::rename(&log_path,&blocked)?;fs::create_dir(&log_path)?;
                    } else {
                        fs::write(&b, b"external")?;
                    }
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read(&a).unwrap(), b"new-a");
        if journal_failure {
            assert_eq!(fs::read(&b).unwrap(), b"old-b");
            fs::remove_dir(&log_path).unwrap();fs::rename(&blocked,&log_path).unwrap();
        } else {
            assert_eq!(fs::read(&b).unwrap(), b"external");
        }
        let mut loaded = Journal::load(&j.path()).unwrap();
        let failures = loaded.rollback();
        assert_eq!(failures.is_empty(), journal_failure);
        assert_eq!(fs::read(&a).unwrap(), b"old-a");
        if !journal_failure {
            fs::write(&b, b"old-b").unwrap();
            assert!(loaded.rollback().is_empty());
        }
    }
}
#[test]
fn sidecar_creation_from_foreign_nfo_and_index_follow_disk_on_save_and_undo() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let dir = root.join("ABC-123");
    fs::create_dir(&dir).unwrap();
    let old = b"<movie><num>ABC-123</num><title>old</title></movie>";
    fs::write(dir.join("foreign.nfo"), old).unwrap();
    fs::write(dir.join("ABC-123.mp4"), b"video").unwrap();
    let lib = kiroku_core::library::Library::open(root).unwrap();
    lib.reindex_dir(&dir).unwrap();
    let p=revision::preview(root,&dir,"ABC-123",serde_json::json!({"title":"new","plot":"new plot","actors":[{"name":"Actor","gender":"unknown"}],"tags":["tag"],"studio":"studio","publisher":"publisher","series":"series","release_date":"2024-01-02","runtime_min":123,"directors":["director"]})).unwrap();
    let mut out = revision::apply(&p).unwrap();
    assert!(out.error.is_none());
    lib.reindex_dir(&dir).unwrap();
    assert_eq!(
        lib.get("ABC-123")
            .unwrap()
            .unwrap()
            .meta
            .fields
            .title
            .as_deref(),
        Some("new")
    );
    assert_eq!(p.after.locked_fields.len(), 10);
    assert!(!dir.join("movie.nfo").exists());
    assert!(out.journal.rollback().is_empty());
    lib.reindex_dir(&dir).unwrap();
    assert_eq!(
        lib.get("ABC-123")
            .unwrap()
            .unwrap()
            .meta
            .fields
            .title
            .as_deref(),
        Some("old")
    );
    assert_eq!(fs::read(dir.join("foreign.nfo")).unwrap(), old);
    assert!(!dir.join("kiroku.json").exists());
}
#[cfg(unix)]
#[test]
fn escaping_backup_paths_and_symlink_targets_are_rejected() {
    let t = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let dir = t.path().join("ABC-123");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("movie.nfo"),
        "<movie><num>ABC-123</num><title>old</title></movie>",
    )
    .unwrap();
    let p = revision::preview(
        t.path(),
        &dir,
        "ABC-123",
        serde_json::json!({"title":"new"}),
    )
    .unwrap();
    let out = revision::apply(&p).unwrap();
    let mut j = out.journal;
    j.replacements[0].backup = Some(outside.path().join("outside"));
    j.save().unwrap();
    assert!(Journal::load(&j.path()).is_err());
    fs::remove_file(dir.join("kiroku.json")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("missing"), dir.join("kiroku.json")).unwrap();
    assert!(revision::preview(
        t.path(),
        &dir,
        "ABC-123",
        serde_json::json!({"title":"new"})
    )
    .is_err());
}
#[cfg(unix)]
#[test]
fn private_file_modes_survive_revision_and_rollback_and_backups_stay_private() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("ABC-123");
    fs::create_dir(&dir).unwrap();
    let m = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    for (name, bytes) in [
        ("movie.nfo", kiroku_core::nfo::to_nfo(&m).into_bytes()),
        ("kiroku.json", serde_json::to_vec(&m).unwrap()),
    ] {
        let p = dir.join(name);
        fs::write(&p, bytes).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let p = revision::preview(
        t.path(),
        &dir,
        "ABC-123",
        serde_json::json!({"title":"new"}),
    )
    .unwrap();
    let mut out = revision::apply(&p).unwrap();
    assert!(out.error.is_none());
    for r in &out.journal.replacements {
        assert_eq!(
            fs::metadata(&r.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        for file in [&r.stage, r.backup.as_ref().unwrap()] {
            assert_eq!(fs::metadata(file).unwrap().permissions().mode() & 0o077, 0);
        }
    }
    assert!(out.journal.rollback().is_empty());
    for name in ["movie.nfo", "kiroku.json"] {
        assert_eq!(
            fs::metadata(dir.join(name)).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
#[test]
fn lock_only_update_does_not_rewrite_identical_nfo() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("ABC-123");
    fs::create_dir(&dir).unwrap();
    let mut m = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    m.fields.title = Some("old".into());
    fs::write(dir.join("movie.nfo"), kiroku_core::nfo::to_nfo(&m)).unwrap();
    fs::write(dir.join("kiroku.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    m.locked_fields.insert("poster_urls".into());
    let p = revision::prepare(t.path(), &dir, "ABC-123", m).unwrap();
    let out = revision::apply(&p).unwrap();
    assert!(out.error.is_none());
    assert_eq!(out.journal.replacements.len(), 1);
    assert!(out.journal.replacements[0].path.ends_with("kiroku.json"));
}

#[test]
fn foreign_nfo_title_edit_preserves_unselected_xml_and_cdata_in_sidecar() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("ABC-123");
    fs::create_dir(&dir).unwrap();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<movie source="external"><title lang="ja">Old</title><sorttitle>Old sort</sorttitle>
<originaltitle><![CDATA[原題 & 原文]]></originaltitle><plot><![CDATA[First <part>]]> and <![CDATA[second & part]]></plot>
<thumb aspect="poster" preview="https://example.test/preview">https://example.test/poster</thumb>
<trailer>https://example.test/trailer</trailer><fileinfo><streamdetails><video><codec>hevc</codec></video></streamdetails></fileinfo>
<unknown attr="keep"><child><![CDATA[opaque <xml>]]></child></unknown><!--keep comment--></movie>"#;
    fs::write(dir.join("foreign.nfo"), xml).unwrap();
    let p = revision::preview(
        t.path(),
        &dir,
        "ABC-123",
        serde_json::json!({"title":"New"}),
    )
    .unwrap();
    assert_eq!(
        p.before.fields.original_title.as_deref(),
        Some("原題 & 原文")
    );
    assert_eq!(
        p.before.fields.plot.as_deref(),
        Some("First <part> and second & part")
    );
    assert_eq!(p.changes.len(), 1);
    assert_eq!(p.changes[0].field, "title");
    let mut out = revision::apply(&p).unwrap();
    assert!(out.error.is_none());
    let saved = fs::read_to_string(dir.join("foreign.nfo")).unwrap();
    assert_eq!(
        saved,
        xml.replace(">Old</title>", ">ABC-123 New</title>")
            .replace(">Old sort</sorttitle>", ">ABC-123 New</sorttitle>")
    );
    let meta: Metadata =
        serde_json::from_slice(&fs::read(dir.join("kiroku.json")).unwrap()).unwrap();
    assert_eq!(meta.fields.original_title, p.before.fields.original_title);
    assert_eq!(meta.fields.plot, p.before.fields.plot);
    assert!(out.journal.rollback().is_empty());
    assert_eq!(fs::read_to_string(dir.join("foreign.nfo")).unwrap(), xml);
}

#[test]
fn foreign_nfo_all_ten_fields_and_locks_use_the_same_lossless_merge() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("ABC-123");
    fs::create_dir(&dir).unwrap();
    let preserved = r#"<originaltitle><![CDATA[原題]]></originaltitle><fileinfo><streamdetails><audio><codec>aac</codec></audio></streamdetails></fileinfo><trailer>https://example.test/t</trailer><unknown keep="yes">opaque</unknown>"#;
    let original=format!("<movie>{preserved}<title>old</title><plot><![CDATA[old plot]]></plot><actor><name>old actor</name></actor><tag>old tag</tag><studio>old studio</studio><publisher>old publisher</publisher><series>old series</series><premiered>2020-01-01</premiered><runtime>1</runtime><director>old director</director></movie>");
    fs::write(dir.join("foreign.nfo"), &original).unwrap();
    let patch = serde_json::json!({"title":"new","plot":"new plot","actors":[{"name":"new actor","gender":"unknown"}],"tags":["new tag"],"studio":"new studio","publisher":"new publisher","series":"new series","release_date":"2026-01-01","runtime_min":99,"directors":["new director"]});
    let p = revision::preview(t.path(), &dir, "ABC-123", patch).unwrap();
    let mut out = revision::apply(&p).unwrap();
    assert!(out.error.is_none());
    let saved = fs::read_to_string(dir.join("foreign.nfo")).unwrap();
    assert!(saved.contains(preserved));
    let parsed = kiroku_core::nfo_read::parse_nfo(&saved, "ABC-123").unwrap();
    assert_eq!(parsed.fields.title, p.after.fields.title);
    assert_eq!(parsed.fields.plot, p.after.fields.plot);
    assert_eq!(parsed.fields.actors, p.after.fields.actors);
    assert_eq!(parsed.fields.tags, p.after.fields.tags);
    assert_eq!(parsed.fields.studio, p.after.fields.studio);
    assert_eq!(parsed.fields.publisher, p.after.fields.publisher);
    assert_eq!(parsed.fields.series, p.after.fields.series);
    assert_eq!(parsed.fields.release_date, p.after.fields.release_date);
    assert_eq!(parsed.fields.runtime_min, p.after.fields.runtime_min);
    assert_eq!(parsed.fields.directors, p.after.fields.directors);
    let mut locked = p.after.clone();
    locked.locked_fields.insert("poster_urls".into());
    let only_lock = revision::prepare(t.path(), &dir, "ABC-123", locked).unwrap();
    let mut lock_out = revision::apply(&only_lock).unwrap();
    assert_eq!(lock_out.journal.replacements.len(), 1);
    assert_eq!(fs::read_to_string(dir.join("foreign.nfo")).unwrap(), saved);
    assert!(lock_out.journal.rollback().is_empty());
    assert!(out.journal.rollback().is_empty());
    assert_eq!(
        fs::read_to_string(dir.join("foreign.nfo")).unwrap(),
        original
    );
}
#[test]
fn unsafe_nfo_is_rejected_at_preview_before_writes() {
    for xml in [
        "<movie><title>broken</movie>",
        "<!DOCTYPE movie [<!ENTITY title 'external'>]><movie><title>&title;</title></movie>",
        "<movie xmlns='custom'><title>old</title></movie>",
        "<movie><title><b>nested title</b></title></movie>",
    ] {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("ABC-123");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("movie.nfo"), xml).unwrap();
        assert!(revision::preview(
            t.path(),
            &dir,
            "ABC-123",
            serde_json::json!({"title":"new"})
        )
        .is_err());
        assert_eq!(fs::read_to_string(dir.join("movie.nfo")).unwrap(), xml);
        assert!(!dir.join("kiroku.json").exists());
        assert!(Journal::list(t.path()).is_empty());
    }
}
#[test]
fn selected_artwork_updates_keep_foreign_thumb_attributes_and_other_fields() {
    let before = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    let mut after = before.clone();
    after.fields.poster_urls = vec!["https://example.test/new".into()];
    let original = r#"<movie><thumb aspect="poster" preview="keep">old-url</thumb><title><![CDATA[keep]]></title><fileinfo>keep info</fileinfo></movie>"#;
    assert_eq!(
        kiroku_core::nfo::merge_revision(original, &before, &after).unwrap(),
        original.replace("old-url", "https://example.test/new")
    );
}
#[test]
fn series_name_merge_and_clear_preserve_unknown_set_art_and_attributes() {
    let mut before = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    before.fields.series = Some("old".into());
    let mut after = before.clone();
    after.fields.series = Some("new".into());
    let original = r#"<movie><set source="external"><name><![CDATA[old]]></name><overview>keep overview</overview><thumb>keep art</thumb></set><title>keep title</title></movie>"#;
    let updated = kiroku_core::nfo::merge_revision(original, &before, &after).unwrap();
    assert!(updated.contains("<name>new</name>"));
    assert!(updated.contains("<overview>keep overview</overview><thumb>keep art</thumb>"));
    let mut cleared = after.clone();
    cleared.fields.series = None;
    let empty = kiroku_core::nfo::merge_revision(&updated, &after, &cleared).unwrap();
    assert!(empty.contains(
        r#"<set source="external"><overview>keep overview</overview><thumb>keep art</thumb></set>"#
    ));
    assert!(!empty.contains("<name>"));
}

#[test]
fn favorites_live_in_the_sidecar_survive_rebuild_and_undo() {
    use kiroku_core::library::{FacetFilter, FacetKind, Library, Query};
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let dir = root.join("ABC-123");
    fs::create_dir(&dir).unwrap();
    let mut m = Metadata { code: "ABC-123".into(), ..Default::default() };
    m.fields.title = Some("old".into());
    let old = serde_json::to_vec(&m).unwrap();
    assert!(!String::from_utf8_lossy(&old).contains("favorite"), "unset favorites are not written");
    fs::write(dir.join("kiroku.json"), &old).unwrap();
    let nfo = b"<movie custom=\"keep\"><title>old</title></movie>";
    fs::write(dir.join("movie.nfo"), nfo).unwrap();
    fs::write(dir.join("ABC-123.mp4"), b"video").unwrap();
    let lib = Library::open(root).unwrap();
    lib.reindex_dir(&dir).unwrap();
    let favorites = |lib: &Library| lib.list(&Query { filters: vec![FacetFilter { kind: FacetKind::Favorite, value: "yes".into() }], ..Default::default() }).unwrap();

    let mut after = m.clone();
    after.favorite = true;
    let p = revision::prepare(root, &dir, "ABC-123", after).unwrap();
    assert!(p.changes.iter().any(|c| c.field == "favorite"));
    let mut out = revision::apply(&p).unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    let saved: Metadata = serde_json::from_slice(&fs::read(dir.join("kiroku.json")).unwrap()).unwrap();
    assert!(saved.favorite);
    assert_eq!(fs::read(dir.join("movie.nfo")).unwrap(), nfo, "favorites never touch the NFO");
    lib.reindex_dir(&dir).unwrap();
    let page = favorites(&lib);
    assert_eq!((page.total, page.items[0].favorite), (1, true));
    assert_eq!(lib.facets().unwrap().favorites, 1);
    lib.rebuild(&tokio_util::sync::CancellationToken::new(), &mut |_, _| {}).unwrap();
    assert_eq!(favorites(&lib).total, 1, "the index is rebuilt from the sidecar");

    assert!(out.journal.rollback().is_empty());
    assert_eq!(fs::read(dir.join("kiroku.json")).unwrap(), old);
    lib.reindex_dir(&dir).unwrap();
    assert_eq!(favorites(&lib).total, 0);
    assert!(!lib.list(&Query::default()).unwrap().items[0].favorite);
}
