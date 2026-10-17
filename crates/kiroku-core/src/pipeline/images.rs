//! Job-local normalized artwork cache. Byte ownership never escapes the job.
use crate::artwork::PreparedImage;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{self, Write},
    path::PathBuf,
};

pub(super) const IMAGE_MEMORY_BUDGET: usize = 64 * 1024 * 1024;
const REPLAY_MEMORY_BUDGET: usize = 8 * 1024 * 1024;
const REPLAY_ENTRY_LIMIT: usize = 8;
const REPLAY_KEY_LIMIT: usize = 4096;

/// One blocking plan's read-only replay of verified prepared images. The job
/// store remains the authority on the first load; replay owns shared bytes only
/// until that plan finishes, fails or is cancelled.
pub(super) struct PlanReplay<'a> {
    source: &'a PreparedImages,
    entries: HashMap<String, PreparedImage>,
    budget: usize,
    resident: usize,
}

impl<'a> PlanReplay<'a> {
    pub fn new(source: &'a PreparedImages, budget: usize) -> Self {
        Self {
            source,
            entries: HashMap::new(),
            budget: budget.min(REPLAY_MEMORY_BUDGET),
            resident: 0,
        }
    }
    pub fn load(&mut self, url: &str) -> io::Result<PreparedImage> {
        if let Some(image) = self.entries.get(url) {
            return Ok(image.clone());
        }
        let image = self.source.load(url)?;
        let size = image.allocation_bytes();
        if self.budget > 0
            && url.len() <= REPLAY_KEY_LIMIT
            && self.entries.len() < REPLAY_ENTRY_LIMIT
            && size <= self.budget - self.resident
        {
            self.resident += size;
            self.entries.insert(url.into(), image.clone());
        }
        Ok(image)
    }
}

enum Stored {
    Memory(PreparedImage),
    Disk {
        path: PathBuf,
        dimensions: (u32, u32),
        digest: String,
    },
}

pub(super) struct PreparedImages {
    entries: HashMap<String, Result<Stored, String>>,
    order: VecDeque<String>,
    selected: HashSet<String>,
    budget: usize,
    resident: usize,
    peak: usize,
    spool: Option<tempfile::TempDir>,
    next_file: usize,
    #[cfg(test)]
    disk_reads: std::sync::atomic::AtomicUsize,
}

impl PreparedImages {
    pub fn new(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            selected: HashSet::new(),
            budget,
            resident: 0,
            peak: 0,
            spool: None,
            next_file: 0,
            #[cfg(test)]
            disk_reads: std::sync::atomic::AtomicUsize::new(0),
        }
    }
    pub fn contains(&self, url: &str) -> bool {
        self.entries.contains_key(url)
    }
    pub fn info(&self, url: &str) -> Result<((u32, u32), &str), &str> {
        match self.entries.get(url) {
            Some(Ok(Stored::Memory(image))) => Ok((image.dimensions, &image.digest)),
            Some(Ok(Stored::Disk {
                dimensions, digest, ..
            })) => Ok((*dimensions, digest)),
            Some(Err(error)) => Err(error),
            None => Err("图片不可用"),
        }
    }
    pub fn insert(&mut self, url: String, image: Result<PreparedImage, String>) {
        debug_assert!(!self.contains(&url));
        let stored = image.and_then(|image| self.store(&url, image).map_err(|e| e.to_string()));
        self.entries.insert(url, stored);
    }
    fn spill(&mut self, image: &PreparedImage) -> io::Result<Stored> {
        if self.spool.is_none() {
            self.spool = Some(
                tempfile::Builder::new()
                    .prefix("kiroku-images-")
                    .tempdir()?,
            );
        }
        let path = self
            .spool
            .as_ref()
            .unwrap()
            .path()
            .join(self.next_file.to_string());
        self.next_file += 1;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&path)?.write_all(image.bytes())?;
        Ok(Stored::Disk {
            path,
            dimensions: image.dimensions,
            digest: image.digest.clone(),
        })
    }
    fn store(&mut self, url: &str, image: PreparedImage) -> io::Result<Stored> {
        let size = image.allocation_bytes();
        if size > self.budget {
            return self.spill(&image);
        }
        while self.resident > self.budget - size {
            let oldest = self
                .order
                .front()
                .expect("resident allocation has an owner")
                .clone();
            let Some(Ok(Stored::Memory(old))) = self.entries.get(&oldest) else {
                unreachable!()
            };
            let old = old.clone();
            let disk = self.spill(&old)?;
            self.resident -= old.allocation_bytes();
            self.entries.insert(oldest, Ok(disk));
            self.order.pop_front();
        }
        self.resident += image.allocation_bytes();
        self.peak = self.peak.max(self.resident);
        self.order.push_back(url.into());
        Ok(Stored::Memory(image))
    }
    pub fn select(&mut self, url: &str) {
        self.selected.insert(url.into());
    }
    pub fn load(&self, url: &str) -> io::Result<PreparedImage> {
        match self.entries.get(url) {
            Some(Ok(Stored::Memory(image))) => Ok(image.clone()),
            Some(Ok(Stored::Disk {
                path,
                dimensions,
                digest,
            })) => {
                #[cfg(test)]
                self.disk_reads
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                PreparedImage::restore(std::fs::read(path)?, *dimensions, digest)
            }
            _ => Err(io::Error::other("图片不可用")),
        }
    }
    pub fn into_legacy(self) -> HashMap<String, Vec<u8>> {
        self.selected
            .iter()
            .filter_map(|url| {
                self.load(url)
                    .ok()
                    .map(|image| (url.clone(), image.bytes().to_vec()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn image(value: u8) -> PreparedImage {
        let mut bytes = io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(40, 60, image::Rgb([value; 3])))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        PreparedImage::normalize(bytes.get_ref()).unwrap()
    }

    #[test]
    fn plan_replay_shares_verified_spill_allocation_after_scratch_is_removed() {
        let mut cache = PreparedImages::new(0);
        cache.insert("shared".into(), Ok(image(10)));
        let mut replay = PlanReplay::new(&cache, 8 * 1024 * 1024);
        let first = replay.load("shared").unwrap();
        let Some(Ok(Stored::Disk { path, .. })) = cache.entries.get("shared") else {
            panic!("must spill")
        };
        std::fs::remove_file(path).unwrap();
        let second = replay.load("shared").unwrap();
        assert_eq!(first.bytes().as_ptr(), second.bytes().as_ptr());
        assert_eq!(first.allocation_bytes(), second.allocation_bytes());
        assert_eq!(first.digest, second.digest);
        drop(replay);
        assert!(
            PlanReplay::new(&cache, 8 * 1024 * 1024)
                .load("shared")
                .is_err(),
            "verified bytes may only replay within their original plan"
        );
    }

    #[test]
    fn plan_replay_respects_zero_small_and_item_budgets_and_skips_large_keys() {
        for budget in [0, 1] {
            let mut cache = PreparedImages::new(0);
            cache.insert("shared".into(), Ok(image(10)));
            let mut replay = PlanReplay::new(&cache, budget);
            replay.load("shared").unwrap();
            let Some(Ok(Stored::Disk { path, .. })) = cache.entries.get("shared") else {
                panic!("must spill")
            };
            std::fs::remove_file(path).unwrap();
            assert!(
                replay.load("shared").is_err(),
                "an image exceeding the plan budget must not stay resident"
            );
        }
        let mut cache = PreparedImages::new(0);
        for i in 0..9 {
            cache.insert(i.to_string(), Ok(image(i)));
        }
        let large_key = "x".repeat(4097);
        cache.insert(large_key.clone(), Ok(image(20)));
        let mut replay = PlanReplay::new(&cache, usize::MAX);
        for i in 0..9 {
            replay.load(&i.to_string()).unwrap();
        }
        replay.load(&large_key).unwrap();
        for entry in cache.entries.values() {
            if let Ok(Stored::Disk { path, .. }) = entry {
                std::fs::remove_file(path).unwrap();
            }
        }
        for i in 0..8 {
            assert!(replay.load(&i.to_string()).is_ok());
        }
        assert!(
            replay.load("8").is_err(),
            "only eight replay entries may remain resident"
        );
        assert!(replay.load(&large_key).is_err());
        assert!(replay.resident <= 8 * 1024 * 1024);
        let mut cache = PreparedImages::new(0);
        cache.insert(large_key.clone(), Ok(image(20)));
        let mut replay = PlanReplay::new(&cache, 8 * 1024 * 1024);
        replay.load(&large_key).unwrap();
        let Some(Ok(Stored::Disk { path, .. })) = cache.entries.get(&large_key) else {
            panic!("must spill")
        };
        std::fs::remove_file(path).unwrap();
        assert!(
            replay.load(&large_key).is_err(),
            "an oversized key must not be retained even when the memo has room"
        );
    }
    #[test]
    fn preparation_spills_in_small_batches_and_shares_resident_bytes() {
        let first = image(10);
        let second = image(20);
        let expected = first.bytes().to_vec();
        let mut cache = PreparedImages::new(second.allocation_bytes());
        cache.insert("first".into(), Ok(first));
        cache.insert("second".into(), Ok(second));
        assert!(
            cache.resident <= cache.budget,
            "resident normalized bytes must respect the budget"
        );
        assert!(cache.peak <= cache.budget);
        assert_eq!(cache.load("first").unwrap().bytes(), expected);
        let a = cache.load("second").unwrap();
        let b = cache.load("second").unwrap();
        assert_eq!(
            a.bytes().as_ptr(),
            b.bytes().as_ptr(),
            "resident fetches must share their allocation"
        );
        let dir = cache.spool.as_ref().unwrap().path().to_owned();
        drop(cache);
        assert!(!dir.exists(), "job scratch files must be removed");
    }

    #[test]
    fn budget_counts_the_owned_allocation_not_only_payload_length() {
        let source = image(30);
        let mut bytes = Vec::with_capacity(4096);
        bytes.extend_from_slice(source.bytes());
        let prepared = PreparedImage::restore(bytes, source.dimensions, &source.digest).unwrap();
        assert!(prepared.bytes().len() < 4096);
        let mut cache = PreparedImages::new(prepared.bytes().len());
        cache.insert("reserved".into(), Ok(prepared));
        assert_eq!(
            cache.resident, 0,
            "unused owned capacity must also consume the memory budget"
        );
        assert_eq!(cache.load("reserved").unwrap().bytes(), source.bytes());
    }

    #[test]
    fn oversized_images_spill_without_residency_and_tampering_is_rejected() {
        let original = image(30);
        let expected = original.bytes().to_vec();
        let mut cache = PreparedImages::new(0);
        cache.insert("large".into(), Ok(original));
        assert_eq!(cache.resident, 0);
        assert_eq!(cache.load("large").unwrap().bytes(), expected);
        let Some(Ok(Stored::Disk { path, .. })) = cache.entries.get("large") else {
            panic!("must spill")
        };
        std::fs::write(path, b"changed").unwrap();
        assert!(cache.load("large").is_err());
    }

    #[test]
    fn spill_failure_keeps_previous_images_and_budget_intact() {
        let first = image(10);
        let second = image(20);
        let expected = first.bytes().to_vec();
        let mut cache = PreparedImages::new(first.allocation_bytes());
        cache.insert("first".into(), Ok(first));
        cache.spool = Some(tempfile::tempdir().unwrap());
        std::fs::remove_dir(cache.spool.as_ref().unwrap().path()).unwrap();
        cache.insert("second".into(), Ok(second));
        assert!(cache.info("second").is_err());
        assert_eq!(cache.load("first").unwrap().bytes(), expected);
        assert!(cache.resident <= cache.budget);
    }

    #[test]
    fn prepared_outputs_share_bytes_and_only_decode_the_source_once() {
        use crate::{
            artwork::{decode_metrics, ImageInput},
            journal::Journal,
            organize::{Op, Plan},
        };
        decode_metrics::take();
        let normalized = image(50);
        let expected = normalized.bytes().to_vec();
        let mut cache = PreparedImages::new(0);
        cache.insert("shared".into(), Ok(normalized));
        let root = tempfile::tempdir().unwrap();
        let plan = Plan {
            ops: ["poster.png", "fanart.png", "original.png"]
                .into_iter()
                .map(|name| Op::Download {
                    url: "shared".into(),
                    path: root.path().join(name),
                    crop: (name == "poster.png").then_some(crate::organize::Crop::PosterRight),
                    stamp: None,
                })
                .collect(),
        };
        let mut journal = Journal::new(root.path());
        let mut replay = PlanReplay::new(&cache, 8 * 1024 * 1024);
        let result = journal.execute_images_observed(
            &plan,
            &mut |url| replay.load(url).map(ImageInput::Prepared),
            &|| false,
            &mut |_| Ok(()),
        );
        assert!(result.error.is_none());
        assert_eq!(result.done.len(), 3);
        assert_eq!(
            cache.disk_reads.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "poster, fanart and original replay one verified disk read"
        );
        assert_eq!(
            decode_metrics::take(),
            1,
            "validated raw outputs must never decode again, including after spill"
        );
        for op in &plan.ops {
            assert_eq!(std::fs::read(op.target()).unwrap(), expected);
        }
        // The legacy/public callback still validates untrusted image bytes.
        let unchecked = Plan {
            ops: vec![Op::Download {
                url: "bad".into(),
                path: root.path().join("bad.png"),
                crop: None,
                stamp: None,
            }],
        };
        let result = journal.execute(&unchecked, &mut |_| Ok(b"<html>".to_vec()), &|| false);
        assert!(result.done.is_empty());
        assert!(!unchecked.ops[0].target().exists());
        assert!(journal.rollback().is_empty());
    }

    #[test]
    fn replay_budget_charges_owned_capacity_and_has_a_hard_eight_mib_ceiling() {
        let source = image(30);
        for (capacity, budget) in [
            (4096, source.bytes().len()),
            (8 * 1024 * 1024 + 1, usize::MAX),
        ] {
            let mut bytes = Vec::with_capacity(capacity);
            bytes.extend_from_slice(source.bytes());
            let prepared =
                PreparedImage::restore(bytes, source.dimensions, &source.digest).unwrap();
            let mut cache = PreparedImages::new(usize::MAX);
            cache.insert("capacity".into(), Ok(prepared));
            let mut replay = PlanReplay::new(&cache, budget);
            let loaded = replay.load("capacity").unwrap();
            assert_eq!(loaded.bytes(), source.bytes());
            assert_eq!(replay.resident, 0, "small payloads in oversized owned allocations must not consume replay budget beyond its ceiling");
            assert!(replay.entries.is_empty());
        }
    }

    #[test]
    #[ignore = "isolated PNG encoding, SHA and spill replay timing; no RSS claims"]
    fn prepared_image_stage_profile() {
        use std::{sync::atomic::Ordering, time::Instant};
        let plane = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(384, 512, |x, y| {
            let mut n = x.wrapping_add(y.wrapping_mul(384)).wrapping_add(1);
            n ^= n << 13;
            n ^= n >> 17;
            n ^= n << 5;
            image::Rgb([(n >> 16) as u8, (n >> 8) as u8, n as u8])
        }));
        let count = 20;
        let start = Instant::now();
        let mut encoded = Vec::new();
        for _ in 0..count {
            let mut cursor = io::Cursor::new(Vec::new());
            plane
                .write_to(&mut cursor, image::ImageFormat::Png)
                .unwrap();
            encoded = cursor.into_inner();
        }
        println!(
            "PNG encoding x{count}: {:?}; payload={} bytes",
            start.elapsed(),
            encoded.len()
        );
        let start = Instant::now();
        for _ in 0..count {
            std::hint::black_box(crate::journal::replacement::digest(&encoded));
        }
        println!("normalized PNG SHA x{count}: {:?}", start.elapsed());
        let image = PreparedImage::normalize(&encoded).unwrap();
        let allocation = image.allocation_bytes();
        let mut cache = PreparedImages::new(0);
        let start = Instant::now();
        cache.insert("shared".into(), Ok(image));
        println!(
            "private spill write x1: {:?}; normalized allocation={allocation} bytes",
            start.elapsed()
        );
        let Some(Ok(Stored::Disk { path, .. })) = cache.entries.get("shared") else {
            panic!("must spill")
        };
        let start = Instant::now();
        for _ in 0..count {
            std::hint::black_box(std::fs::read(path).unwrap());
        }
        println!(
            "private scratch read without SHA x{count}: {:?}",
            start.elapsed()
        );
        let start = Instant::now();
        for _ in 0..count {
            std::hint::black_box(cache.load("shared").unwrap());
        }
        let uncached = start.elapsed();
        assert_eq!(cache.disk_reads.swap(0, Ordering::Relaxed), count);
        let mut replay = PlanReplay::new(&cache, REPLAY_MEMORY_BUDGET);
        let start = Instant::now();
        for _ in 0..count {
            std::hint::black_box(replay.load("shared").unwrap());
        }
        let cached = start.elapsed();
        assert_eq!(cache.disk_reads.load(Ordering::Relaxed), 1);
        println!("read + digest restore x{count}: {uncached:?} -> plan replay {cached:?}; actual disk reads {count} -> 1; replay retained={} bytes", replay.resident);
        println!("Timings are synthetic debug-build phase costs; exclude RSS, OS file-cache claims, index SHA and publication checkpoints.");
    }

    #[test]
    #[ignore = "explicit synthetic memory/CPU measurement"]
    fn normalized_cache_budget_measurement() {
        use crate::artwork::decode_metrics;
        let pixels = image::RgbImage::from_fn(384, 512, |x, y| {
            let mut n = x.wrapping_add(y.wrapping_mul(384)).wrapping_add(1);
            n ^= n << 13;
            n ^= n >> 17;
            n ^= n << 5;
            image::Rgb([(n >> 16) as u8, (n >> 8) as u8, n as u8])
        });
        let mut source = io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(pixels)
            .write_to(&mut source, image::ImageFormat::Png)
            .unwrap();
        let count = 100;
        // Reference the previous preparation: normalized candidate cache plus a
        // cloned selected-output cache, with a full decode to obtain dimensions.
        let start = std::time::Instant::now();
        let mut previous = Vec::new();
        let mut old_payload = 0;
        for _ in 0..count {
            let bytes = crate::artwork::normalize(source.get_ref()).unwrap();
            image::load_from_memory(&bytes).unwrap();
            let selected = bytes.clone();
            old_payload += bytes.capacity() + selected.capacity();
            previous.push((selected, bytes));
        }
        let old_time = start.elapsed();
        drop(previous);
        println!("100 images: previous retained allocation={old_payload} bytes; reference elapsed={old_time:?}");
        for budget in [IMAGE_MEMORY_BUDGET, 2 * 1024 * 1024] {
            decode_metrics::take();
            let start = std::time::Instant::now();
            let mut cache = PreparedImages::new(budget);
            for i in 0..count {
                cache.insert(
                    i.to_string(),
                    Ok(PreparedImage::normalize(source.get_ref()).unwrap()),
                );
            }
            let elapsed = start.elapsed();
            assert!(cache.peak <= budget);
            assert_eq!(decode_metrics::take(), count);
            println!("bounded peak={} bytes; budget={budget} bytes; preparation decodes=200 -> 100; elapsed={elapsed:?}", cache.peak);
        }
        println!("Allocation accounting excludes download/decode working memory and OS file cache; elapsed comparison includes spill I/O.");
    }
}
