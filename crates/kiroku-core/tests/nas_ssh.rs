//! Explicit opt-in integration test: only newly created neutral files on the NAS.
use kiroku_core::{
    journal::{Checkpoint, Journal},
    nas_ops::{self, Config},
    organize::{Op, Plan},
};
use std::{fs, io, path::PathBuf};
#[test]
#[ignore = "requires explicit NAS SSH host/local/remote test root variables"]
fn ssh_move_is_linked_exclusive_recoverable_and_mapping_checked() {
    let base = PathBuf::from(std::env::var_os("KIROKU_NAS_TEST_ROOT").unwrap());
    let remote = PathBuf::from(std::env::var_os("KIROKU_NAS_REMOTE_ROOT").unwrap());
    let host = std::env::var("KIROKU_NAS_SSH_HOST").unwrap();
    let temp = tempfile::Builder::new()
        .prefix(".kiroku-ssh-regression-")
        .tempdir_in(&base)
        .unwrap();
    let cfg = Config {
        host,
        local_root: temp.path().into(),
        remote_root: remote.join(temp.path().file_name().unwrap()),
    };
    nas_ops::configure(Some(cfg.clone())).unwrap();
    assert_eq!(nas_ops::diagnose(&|| false).unwrap().state, "verified");
    let source = temp.path().join("neutral 'quoted' cafe\u{301}.mp4");
    let bytes = vec![17; 9 * 1024 * 1024];
    fs::write(&source, &bytes).unwrap();
    let root = temp.path().join("library");
    let target = root.join("ABC-001.mp4");
    let plan = Plan {
        ops: vec![Op::MoveFile {
            from: source.clone(),
            to: target.clone(),
        }],
    };
    let mut journal = Journal::new(&root);
    let started = std::time::Instant::now();
    let out = journal.execute(&plan, &mut |_| unreachable!(), &|| false);
    assert!(out.error.is_none(), "{:?}", out.error);
    assert!(!nas_ops::exists(&source).unwrap().unwrap());
    assert!(nas_ops::same_file(&target, &journal.proofs[&0].stage)
        .unwrap()
        .unwrap());
    assert!(journal.proofs[&0].published_identity.is_none());
    let mut saved = Journal::load(&journal.path()).unwrap();
    assert!(saved.rollback().is_empty());
    assert_eq!(fs::read(&source).unwrap(), bytes);
    println!("NAS move + checked rollback: {:?}", started.elapsed());
    // Remote-created composed names must remain accessible through macOS SMB.
    let code = kiroku_core::code::parse_code("ABC-001.mp4").unwrap();
    let meta = kiroku_core::metadata::Metadata {
        code: code.id.clone(),
        fields: kiroku_core::metadata::PartialMetadata {
            title: Some("Café ガイド".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let unicode_plan = kiroku_core::organize::plan_with(
        &source,
        &code,
        &meta,
        &root,
        "{code} {title}",
        true,
        kiroku_core::organize::PlanOpts {
            file_template: "{code} {title}".into(),
            ..Default::default()
        },
    );
    assert!(unicode_plan.ops.len() >= 3);
    let mut unicode_journal = Journal::new(&root);
    let out = unicode_journal.execute(&unicode_plan, &mut |_| unreachable!(), &|| false);
    assert!(
        out.error.is_none(),
        "{:?}; done={:?}; pending={:?}",
        out.error,
        out.done,
        unicode_journal.pending
    );
    for op in &unicode_plan.ops {
        assert!(op.target().exists(), "remote target invisible over SMB");
    }
    let mut loaded = Journal::load(&unicode_journal.path()).unwrap();
    assert!(loaded.rollback().is_empty());
    assert!(source.exists());
    for op in &unicode_plan.ops {
        assert!(!nas_ops::exists(op.target()).unwrap().unwrap());
    }
    for point in [
        Checkpoint::Staged,
        Checkpoint::Published,
        Checkpoint::SourceRemoved,
    ] {
        let mut j = Journal::new(&root);
        let out = j.execute_observed(&plan, &mut |_| unreachable!(), &|| false, &mut |p| {
            if p == point {
                Err(io::Error::other("simulated interruption"))
            } else {
                Ok(())
            }
        });
        assert!(out.error.is_some());
        let mut recovered = Journal::load(&j.path()).unwrap();
        assert!(recovered.rollback().is_empty());
        assert_eq!(fs::read(&source).unwrap(), bytes);
        assert!(!nas_ops::exists(&target).unwrap().unwrap());
    }
    let mut j = Journal::new(&root);
    let result = j.execute_observed(&plan, &mut |_| unreachable!(), &|| false, &mut |p| {
        if p == Checkpoint::Staged {
            fs::write(&target, b"external")?;
        }
        Ok(())
    });
    assert!(result.error.is_some());
    assert_eq!(fs::read(&target).unwrap(), b"external");
    assert!(source.exists());
    fs::remove_file(&target).unwrap();
    j.recover_pending().unwrap();
    let wrong = Config {
        remote_root: cfg.remote_root.join("wrong"),
        ..cfg
    };
    nas_ops::configure(Some(wrong)).unwrap();
    let mut j = Journal::new(&root);
    let result = j.execute(
        &Plan {
            ops: vec![Op::MoveFile {
                from: source.clone(),
                to: root.join("wrong.mp4"),
            }],
        },
        &mut |_| unreachable!(),
        &|| false,
    );
    assert!(result.error.is_some());
    assert!(source.exists());
    nas_ops::configure(None).unwrap();
}
