//! Only the existing neutral localhost fixture is used by these controls.
use super::*;
use kiroku_core::{
    journal::{Checkpoint, Journal, JournalItem, JournalStatus},
    organize::Plan,
    pipeline,
};

#[tauri::command]
pub(super) async fn bridge_resume_prepare(
    app: AppHandle,
    probe: State<'_, Arc<Probe>>,
) -> Result<Value, String> {
    let probe = probe.inner().clone();
    let state = app.state::<AppState>();
    if state.job_running() || Settings::default_path() != probe.config_path {
        return Err("resume fixture not isolated or busy".into());
    }
    let (items, input, mut settings) = {
        let network = probe.network.lock().unwrap();
        let fixture = network.as_ref().ok_or("network fixture absent")?;
        (
            fixture.items.clone(),
            fixture.input_root(),
            fixture.settings(state.settings()),
        )
    };
    settings.organize_mode = kiroku_core::placement::OrganizeMode::Move;
    settings.cleanup_source = false;
    settings.source_dirs = vec![input.display().to_string()];
    settings.dir_template = "{code}".into();
    settings.file_template = "{code}".into();
    let scraper = pipeline::Scraper::new(settings.clone())
        .map_err(|e| e.to_string())?
        .with_import_cache(kiroku_core::import_cache::cache_path(&probe.config_path));
    let codes = items
        .iter()
        .map(|i| i.code.clone().unwrap())
        .collect::<Vec<_>>();
    let cancel = CancellationToken::new();
    let results =
        pipeline::scrape_missing(&scraper, &codes, &Default::default(), &cancel, &|_| {}).await;
    let metadata = results
        .into_iter()
        .map(|(id, result)| result.map(|m| (id, m)).map_err(|e| e.message()))
        .collect::<Result<std::collections::HashMap<_, _>, _>>()?;
    let pairs = items
        .iter()
        .map(|i| (i, i.code.as_ref().unwrap()))
        .collect::<Vec<_>>();
    let mut plans = pipeline::build_plans(
        &pairs,
        &metadata,
        &probe.root,
        "{code}",
        settings.plan_opts(),
    );
    if plans.len() != 3 || plans.iter().any(|p| !p.selected) {
        return Err("unexpected resume plans".into());
    }
    let first = plans.remove(0);
    let second = plans.remove(0);
    let out = pipeline::apply(
        vec![first],
        &scraper.http,
        &probe.root,
        None,
        &cancel,
        &|_| {},
    )
    .await;
    if !out.failed.is_empty() || out.applied.len() != 1 {
        return Err("could not prepare completed first item".into());
    }
    let id = out.journal.id.clone();
    let result_settings = settings.clone();
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Reopen from disk before creating the interruption; no in-memory plan survives.
        let mut journal = Journal::load(&out.journal.path()).map_err(|e| e.to_string())?;
        journal.status = JournalStatus::Partial;
        journal.items.push(JournalItem {
            code: second.code.clone(),
            dir: second.to.parent().unwrap().into(),
        });
        let result = journal.execute_observed(
            &Plan { ops: second.ops },
            &mut |_| unreachable!(),
            &|| false,
            &mut |point| {
                if point == Checkpoint::Intent {
                    Err(std::io::Error::other("neutral test interruption"))
                } else {
                    Ok(())
                }
            },
        );
        if result.error.is_none() || journal.pending.is_none() {
            return Err("pending intent not persisted".to_string());
        }
        settings
            .save(&probe.config_path)
            .map_err(|e| e.to_string())?;
        let state = handle.state::<AppState>();
        *state.settings.lock().unwrap() = settings;
        *state.session() = Default::default();
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(json!({"settings":result_settings,"journalId":id}))
}

#[tauri::command]
pub(super) async fn bridge_resume_verify(
    app: AppHandle,
    probe: State<'_, Arc<Probe>>,
    journal_id: String,
    phase: String,
) -> Result<(), String> {
    let probe = probe.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        if state.job_running() {
            return Err("resume task slot still occupied".into());
        }
        let input = probe.network.lock().unwrap().as_ref().unwrap().input_root();
        let index = kiroku_core::library::Library::open(&probe.root).map_err(|e| e.to_string())?;
        for (i, code) in ["NET-101", "NET-102", "NET-103"].iter().enumerate() {
            let committed = phase == "applied" || (phase == "preserved" && i == 0);
            let source = input.join(format!("{code}.mp4"));
            let target = probe.root.join(code).join(format!("{code}.mp4"));
            if committed {
                if source.exists()
                    || fs::read(target).map_err(|e| e.to_string())? != b"synthetic network video"
                    || index.get(code).map_err(|e| e.to_string())?.is_none()
                {
                    return Err(format!("missing committed media/index {code}"));
                }
            } else if fs::read(source).map_err(|e| e.to_string())? != b"synthetic network video"
                || target.exists()
                || index.get(code).map_err(|e| e.to_string())?.is_some()
            {
                return Err(format!("source or index not restored {code}"));
            }
        }
        let journal = Journal::load(
            &probe
                .root
                .join(".kiroku")
                .join(format!("journal-{journal_id}.json")),
        )
        .map_err(|e| e.to_string())?;
        if journal.pending.is_some()
            || journal.status
                != if phase == "restored" {
                    JournalStatus::RolledBack
                } else {
                    JournalStatus::Partial
                }
        {
            return Err("original journal state changed incorrectly".into());
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
