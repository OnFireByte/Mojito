//! Settings persistence: one JSON file, durable writes, ArcSwap hot reload.
//! The settings window is the only writer; the engine reads per event.

use crate::config::Config;
use crate::labels::LabelMemory;
use arc_swap::ArcSwap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// How long label patches coalesce before hitting disk. Long enough to merge
/// a burst of toggles, short enough that a crash loses nothing that matters.
const LABEL_DEBOUNCE_MS: u64 = 300;

struct Pending {
    patch: LabelMemory,
    scheduled: bool,
}

pub struct ConfigStore(Arc<Shared>);

struct Shared {
    path: PathBuf,
    current: Arc<ArcSwap<Config>>,
    /// Single writer: durable writes (and the debounced label flush) are
    /// serialized so versions never go backwards.
    writer: Mutex<()>,
    pending: Mutex<Pending>,
}

impl ConfigStore {
    /// A missing or unreadable file loads defaults and leaves the original
    /// untouched. An unreadable file is preserved as
    /// `settings.json.unreadable-<sha256>` before any save overwrites it.
    pub fn open(path: PathBuf) -> Self {
        ConfigStore(Arc::new(Shared {
            current: Arc::new(ArcSwap::from_pointee(load(&path))),
            path,
            writer: Mutex::new(()),
            pending: Mutex::new(Pending {
                patch: LabelMemory::default(),
                scheduled: false,
            }),
        }))
    }

    pub fn handle(&self) -> Arc<ArcSwap<Config>> {
        self.0.current.clone()
    }

    /// Durable write (fsync file and directory, one mutex), then swap the
    /// Arc so the engine sees the new config on its next event. The version
    /// is bumped here, not by callers, so every accepted save is hot-reloaded.
    pub fn save(&self, next: Config) -> Result<(), std::io::Error> {
        self.0.persist(next)
    }

    /// Debounced merge of sticky label memory after a Live activation. The
    /// patch lands in a pending set; one background flush writes the merged
    /// config through the same durable path as `save`.
    pub fn merge_labels(&self, patch: LabelMemory) {
        let mut pending = self.0.pending.lock().unwrap();
        pending.patch.0.extend(patch.0);
        if pending.scheduled {
            return;
        }
        pending.scheduled = true;
        drop(pending);
        let shared = self.0.clone();
        let spawned = std::thread::Builder::new()
            .name("mojito-label-merge".into())
            .spawn(move || shared.flush_later());
        if spawned.is_err() {
            self.0.pending.lock().unwrap().scheduled = false;
        }
    }

    /// Label memory is engine-owned: a settings save from another process
    /// must not roll it back to a stale snapshot.
    pub fn with_disk_labels(&self, mut next: Config) -> Config {
        if let Ok(bytes) = std::fs::read(&self.0.path) {
            let (disk, _) = crate::config::parse(&bytes);
            next.label_memory = disk.label_memory;
        }
        next
    }

    /// Re-read the file another process saved and swap it in. The version
    /// moves past the current one so the engine rebuilds its tables, and
    /// label memory stays engine-owned (the file wins for labels), so a
    /// settings save never clobbers labels merged while the editor was
    /// open.
    pub fn reload(&self) -> Result<(), std::io::Error> {
        let bytes = match std::fs::read(&self.0.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };
        let (mut next, _) = crate::config::parse(&bytes);
        let _guard = self.0.writer.lock().unwrap();
        next.version = self.0.current.load().version.max(next.version) + 1;
        self.0.current.store(Arc::new(next));
        Ok(())
    }
}

impl Shared {
    fn flush_later(&self) {
        std::thread::sleep(std::time::Duration::from_millis(LABEL_DEBOUNCE_MS));
        let patch = {
            let mut pending = self.pending.lock().unwrap();
            pending.scheduled = false;
            std::mem::take(&mut pending.patch)
        };
        if patch.0.is_empty() {
            return;
        }
        let mut next = (*self.current.load_full()).clone();
        next.label_memory.0.extend(patch.0);
        if let Err(error) = self.persist(next) {
            eprintln!(
                "mojito: label memory write to {}: {error}",
                self.path.display()
            );
        }
    }

    fn persist(&self, mut next: Config) -> Result<(), std::io::Error> {
        let _guard = self.writer.lock().unwrap();
        next.version = self.current.load().version.wrapping_add(1);
        let bytes = serde_json::to_vec_pretty(&next)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self
            .path
            .with_extension(format!("json.tmp-{}", std::process::id()));
        {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path)?;
        // Directory entry durability is best-effort; the file itself is synced.
        if let Some(dir) = self.path.parent()
            && let Ok(dir) = std::fs::File::open(dir)
        {
            let _ = dir.sync_all();
        }
        self.current.store(Arc::new(next));
        Ok(())
    }
}

fn load(path: &Path) -> Config {
    let Ok(bytes) = std::fs::read(path) else {
        return Config::default();
    };
    if bytes.is_empty() {
        return Config::default();
    }
    let unreadable = serde_json::from_slice::<serde_json::Value>(&bytes)
        .map(|v| !v.is_object())
        .unwrap_or(true);
    if unreadable {
        preserve_unreadable(path, &bytes);
    }
    crate::config::parse(&bytes).0
}

/// Copy the raw bytes aside, named by their digest, so a corrupt settings
/// file is never silently destroyed by the next save.
fn preserve_unreadable(path: &Path, bytes: &[u8]) {
    let mut backup = path.as_os_str().to_os_string();
    backup.push(format!(".unreadable-{}", hex(&sha256(bytes))));
    let _ = std::fs::write(PathBuf::from(backup), bytes);
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 (FIPS 180-4), hand-rolled: the only hash the unreadable-backup
/// name needs, and no new dependency is allowed in this crate.
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    for block in message.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d) = (h[0], h[1], h[2], h[3]);
        let (mut e, mut f, mut g, mut hh) = (h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut digest = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        digest[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PinnedApp, parse};
    use crate::key::PhysicalKey;
    use crate::labels::LabelMemory;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mojito-core-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sha256_matches_known_vectors() {
        let hex = |d: &[u8]| hex(&sha256(d));
        assert_eq!(
            hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn open_without_a_file_loads_defaults() {
        let dir = temp_dir("open-missing");
        let store = ConfigStore::open(dir.join("settings.json"));
        assert_eq!(store.handle().load().tap_max_ms, 250);
        assert_eq!(store.handle().load().version, 0);
    }

    #[test]
    fn open_with_a_file_parses_it() {
        let dir = temp_dir("open-parse");
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"tap_max_ms": 120}"#).unwrap();
        let store = ConfigStore::open(path);
        assert_eq!(store.handle().load().tap_max_ms, 120);
    }

    #[test]
    fn corrupt_file_is_preserved_and_defaults_load() {
        let dir = temp_dir("corrupt");
        let path = dir.join("settings.json");
        let bad = b"{this is not json at all";
        std::fs::write(&path, bad).unwrap();
        let store = ConfigStore::open(path.clone());
        assert_eq!(**store.handle().load(), Config::default());
        let backup = path.with_extension(format!("json.unreadable-{}", hex(&sha256(bad))));
        assert_eq!(std::fs::read(&backup).unwrap(), bad);
        // The original is untouched until a save overwrites it.
        assert_eq!(std::fs::read(&path).unwrap(), bad);
    }

    #[test]
    fn save_round_trips_and_bumps_the_version() {
        let dir = temp_dir("save");
        let path = dir.join("settings.json");
        let store = ConfigStore::open(path.clone());
        let next = Config {
            pinned: vec![PinnedApp {
                name: "Finder".into(),
                bundle_id: "com.apple.finder".into(),
                key: PhysicalKey(0x18),
            }],
            tap_max_ms: 200,
            ..Default::default()
        };
        store.save(next.clone()).unwrap();
        assert_eq!(store.handle().load().version, 1);
        assert_eq!(store.handle().load().pinned, next.pinned);
        let (parsed, warnings) = parse(&std::fs::read(&path).unwrap());
        assert!(warnings.is_empty());
        assert_eq!(parsed.tap_max_ms, 200);
        assert_eq!(parsed.pinned.len(), 1);
        assert_eq!(parsed.version, 1);
        store.save(next).unwrap();
        assert_eq!(store.handle().load().version, 2);
    }

    #[test]
    fn merged_labels_land_on_disk_after_the_debounce() {
        let dir = temp_dir("merge");
        let path = dir.join("settings.json");
        let store = ConfigStore::open(path.clone());
        store.save(Config::default()).unwrap();
        let mut memory = LabelMemory::default();
        memory.0.insert("com.a".into(), PhysicalKey(3));
        store.merge_labels(memory.clone());
        store.merge_labels({
            let mut more = LabelMemory::default();
            more.0.insert("com.b".into(), PhysicalKey(2));
            more
        });
        std::thread::sleep(std::time::Duration::from_millis(LABEL_DEBOUNCE_MS + 300));
        let (config, warnings) = parse(&std::fs::read(&path).unwrap());
        assert!(warnings.is_empty());
        assert_eq!(config.label_memory.0.get("com.a"), Some(&PhysicalKey(3)));
        assert_eq!(config.label_memory.0.get("com.b"), Some(&PhysicalKey(2)));
        assert_eq!(config.version, 2);
    }
}
