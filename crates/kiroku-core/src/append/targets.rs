//! One request-scoped disk walk. Only requested codes and matching paths survive it.
use super::{check_cancel, confined_path, read_meta};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;

pub(crate) struct Targets {
    alias: PathBuf,
    root: PathBuf,
    identity: same_file::Handle,
    matches: HashMap<String, Vec<PathBuf>>,
}
pub(super) fn key(code: &str) -> String {
    crate::code::parse_code(code)
        .map(|code| code.id)
        .unwrap_or_else(|| code.to_owned())
}
impl Targets {
    pub fn capture<'a>(
        root: &Path,
        codes: impl IntoIterator<Item = &'a str>,
        cancel: &CancellationToken,
    ) -> io::Result<Self> {
        Self::capture_observed(root, codes, cancel, &|_| {})
    }
    fn capture_observed<'a>(
        root: &Path,
        codes: impl IntoIterator<Item = &'a str>,
        cancel: &CancellationToken,
        observe: &dyn Fn(&Path),
    ) -> io::Result<Self> {
        check_cancel(cancel)?;
        let requested: HashSet<_> = codes.into_iter().map(key).collect();
        let alias = std::path::absolute(root)?;
        let identity = same_file::Handle::from_path(&alias)?;
        let root = alias.canonicalize()?;
        let mut snapshot = Self {
            alias,
            root,
            identity,
            matches: HashMap::new(),
        };
        snapshot.verify()?;
        fs::read_dir(&snapshot.root)?;
        #[cfg(test)]
        stats::walk();
        let mut found: HashMap<String, HashSet<PathBuf>> = HashMap::new();
        for entry in WalkDir::new(&snapshot.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| entry.file_name() != ".kiroku")
        {
            check_cancel(cancel)?;
            let entry = entry.map_err(io::Error::other)?;
            observe(entry.path());
            check_cancel(cancel)?;
            #[cfg(test)]
            stats::entry();
            if entry.file_type().is_file()
                && (entry.file_name() == "kiroku.json"
                    || entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("nfo")))
            {
                let directory = entry.path().parent().unwrap();
                confined_path(&snapshot.root, entry.path())?;
                #[cfg(test)]
                stats::metadata();
                let metadata = read_meta(directory).map_err(|error| {
                    io::Error::new(error.kind(), format!("{}：{error}", directory.display()))
                })?;
                let code = key(&metadata.code);
                if requested.contains(&code) {
                    found
                        .entry(code)
                        .or_default()
                        .insert(directory.to_path_buf());
                }
            }
        }
        check_cancel(cancel)?;
        snapshot.verify()?;
        snapshot.matches = found
            .into_iter()
            .map(|(code, paths)| {
                let mut paths: Vec<_> = paths.into_iter().collect();
                paths.sort();
                (code, paths)
            })
            .collect();
        Ok(snapshot)
    }
    pub fn verify(&self) -> io::Result<()> {
        if same_file::Handle::from_path(&self.alias)? != self.identity
            || same_file::Handle::from_path(&self.root)? != self.identity
        {
            return Err(io::Error::other("媒体库身份已变化，请重新生成计划"));
        }
        Ok(())
    }
    pub fn get(&self, code: &str, cancel: &CancellationToken) -> io::Result<&[PathBuf]> {
        check_cancel(cancel)?;
        self.verify()?;
        Ok(self
            .matches
            .get(&key(code))
            .map(Vec::as_slice)
            .unwrap_or(&[]))
    }
}
#[cfg(test)]
pub(crate) mod stats {
    thread_local! {static COUNTS:std::cell::Cell<(usize,usize,usize)>=const{std::cell::Cell::new((0,0,0))};}
    pub(super) fn walk() {
        COUNTS.with(|value| {
            let (a, b, c) = value.get();
            value.set((a + 1, b, c));
        });
    }
    pub(super) fn entry() {
        COUNTS.with(|value| {
            let (a, b, c) = value.get();
            value.set((a, b + 1, c));
        });
    }
    pub(super) fn metadata() {
        COUNTS.with(|value| {
            let (a, b, c) = value.get();
            value.set((a, b, c + 1));
        });
    }
    pub(crate) fn measure<T>(work: impl FnOnce() -> T) -> (T, (usize, usize, usize)) {
        COUNTS.with(|value| value.set((0, 0, 0)));
        let result = work();
        (result, COUNTS.with(std::cell::Cell::get))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn movie(root: &Path, name: &str, code: &str) -> PathBuf {
        let directory = root.join(name);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("kiroku.json"),
            serde_json::to_vec(&crate::Metadata {
                code: code.into(),
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
        directory
    }
    #[test]
    fn requested_candidates_are_sorted_deduplicated_fresh_and_identity_bound() {
        let temp = tempfile::tempdir().unwrap();
        let a = movie(temp.path(), "a/movie", "FC2-PPV-001");
        let b = movie(temp.path(), "a-dup", "FC2-001");
        movie(temp.path(), "unrelated", "ABC-999");
        fs::write(a.join("movie.nfo"), b"ignored because sidecar wins").unwrap();
        let cancel = CancellationToken::new();
        let snapshot = Targets::capture(temp.path(), ["FC2-001", "ABC-888"], &cancel).unwrap();
        let mut expected = vec![a.canonicalize().unwrap(), b.canonicalize().unwrap()];
        expected.sort();
        assert_eq!(snapshot.get("FC2-001", &cancel).unwrap(), expected);
        assert_eq!(snapshot.matches.len(), 1);
        movie(temp.path(), "later", "ABC-888");
        assert!(snapshot.get("ABC-888", &cancel).unwrap().is_empty());
        assert_eq!(
            Targets::capture(temp.path(), ["ABC-888"], &cancel)
                .unwrap()
                .get("ABC-888", &cancel)
                .unwrap()
                .len(),
            1
        );
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("library");
        fs::create_dir(&root).unwrap();
        let bound = Targets::capture(&root, ["ABC-111"], &cancel).unwrap();
        fs::rename(&root, parent.path().join("offline")).unwrap();
        fs::create_dir(&root).unwrap();
        assert!(bound.get("ABC-111", &cancel).is_err());
        assert!(!root.join(".kiroku").exists());
    }
    #[test]
    fn cached_discovery_does_not_authorize_a_changed_movie_or_trust_sqlite() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let directory = movie(&root, "movie", "ABC-111");
        fs::write(directory.join("ABC-111-CD1.mp4"), b"old").unwrap();
        fs::create_dir(root.join(".kiroku")).unwrap();
        fs::write(root.join(".kiroku/library.db"), b"not a database").unwrap();
        let cancel = CancellationToken::new();
        let snapshot = Targets::capture(&root, ["ABC-111"], &cancel).unwrap();
        assert_eq!(snapshot.get("ABC-111", &cancel).unwrap().len(), 1);
        fs::write(
            directory.join("kiroku.json"),
            serde_json::to_vec(&crate::Metadata {
                code: "ABC-222".into(),
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
        let input = temp.path().join("ABC-111-CD2.mp4");
        fs::write(&input, b"new").unwrap();
        assert!(super::super::preview_from_targets(
            &root,
            "ABC-111",
            &[input],
            None,
            None,
            &cancel,
            snapshot.get("ABC-111", &cancel).unwrap()
        )
        .unwrap_err()
        .to_string()
        .contains("番号已变化"));
        let fresh = Targets::capture(&root, ["ABC-111", "ABC-222"], &cancel).unwrap();
        assert!(fresh.get("ABC-111", &cancel).unwrap().is_empty());
        assert_eq!(fresh.get("ABC-222", &cancel).unwrap().len(), 1);
    }

    #[test]
    fn cancellation_or_root_loss_mid_walk_never_returns_partial_candidates() {
        for disconnect in [false, true] {
            let parent = tempfile::tempdir().unwrap();
            let root = parent.path().join("library");
            movie(&root, "first", "ABC-111");
            let cancel = CancellationToken::new();
            let fired = std::cell::Cell::new(false);
            let result = Targets::capture_observed(&root, ["ABC-111"], &cancel, &|path| {
                if path.file_name().is_some_and(|name| name == "kiroku.json")
                    && !fired.replace(true)
                {
                    if disconnect {
                        fs::rename(&root, parent.path().join("offline")).unwrap();
                    } else {
                        cancel.cancel();
                    }
                }
            });
            assert!(result.is_err());
            if disconnect {
                assert!(!root.exists());
            } else {
                assert_eq!(result.err().unwrap().kind(), io::ErrorKind::Interrupted);
            }
        }
    }
}
