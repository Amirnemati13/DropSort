//! Filesystem operations are kept independent from the GUI.
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

pub const CATEGORIES: [&str; 7] = [
    "Documents",
    "Images",
    "Videos",
    "Audio",
    "Archives",
    "Software",
    "Other",
];

pub fn category(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" | "csv" | "rtf"
        | "odt" | "ods" | "epub" => "Documents",
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "svg" | "ico"
        | "heic" | "avif" | "raw" => "Images",
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "m4v" | "mpeg" | "mpg" => "Videos",
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "wma" | "opus" | "aiff" => "Audio",
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" => "Archives",
        "exe" | "msi" | "msix" | "appx" | "msixbundle" | "appxbundle" => "Software",
        _ => "Other",
    }
}

fn regular(path: &Path) -> io::Result<()> {
    let m = fs::symlink_metadata(path)?;
    if !m.file_type().is_file() {
        return Err(io::Error::other(
            "Only regular files are supported (no folders or links).",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other(
                "Reparse points and cloud placeholders are not supported; use a local file.",
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct PlannedMove {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub category: &'static str,
    pub bytes: u64,
}

/// Preview reserves names across the batch, without creating any directories.
pub fn plan(files: &[PathBuf], root: &Path) -> io::Result<Vec<PlannedMove>> {
    if !root.is_dir() {
        return Err(io::Error::other("Choose an existing destination folder."));
    }
    let root = fs::canonicalize(root)?;
    let mut reserved = HashSet::new();
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for source in files {
        regular(source)?;
        let source = fs::canonicalize(source)?;
        if !seen.insert(key(&source)) {
            continue;
        }
        let category = category(&source);
        let dir = root.join(category);
        // Refuse redirected category folders so the preview cannot conceal a link target.
        if let Ok(m) = fs::symlink_metadata(&dir) {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if m.file_attributes() & 0x400 != 0 {
                    return Err(io::Error::other(
                        "A category folder is a link or reparse point.",
                    ));
                }
            }
            if !m.is_dir() || m.file_type().is_symlink() {
                return Err(io::Error::other("A category path is not a regular folder."));
            }
        }
        let name = source
            .file_name()
            .ok_or_else(|| io::Error::other("Invalid filename."))?;
        let base = dir.join(name);
        if source == base {
            continue;
        }
        let mut destination = base.clone();
        let mut n = 1;
        while destination.try_exists()? || reserved.contains(&key(&destination)) {
            let mut name = base.file_stem().unwrap_or_default().to_os_string();
            name.push(format!(" ({n})"));
            if let Some(ext) = base.extension() {
                name.push(".");
                name.push(ext);
            }
            destination = dir.join(name);
            n += 1;
        }
        reserved.insert(key(&destination));
        result.push(PlannedMove {
            bytes: fs::metadata(&source)?.len(),
            source,
            destination,
            category,
        });
    }
    Ok(result)
}

fn key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn fingerprint(path: &Path) -> io::Result<[u8; 32]> {
    regular(path)?;
    let mut f = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hash.finalize().into())
}

/// Windows performs the move without REPLACE_EXISTING. COPY_ALLOWED supports other drives.
#[cfg(windows)]
fn move_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_COPY_ALLOWED, MOVEFILE_WRITE_THROUGH,
    };
    let a: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let b: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: both pointers refer to live, NUL-terminated UTF-16 paths for this call.
    if unsafe {
        MoveFileExW(
            a.as_ptr(),
            b.as_ptr(),
            MOVEFILE_COPY_ALLOWED | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if source.try_exists()? {
        return Err(io::Error::other("Windows copied the file but could not remove the original. Both copies were kept; inspect them manually."));
    }
    Ok(())
}

#[cfg(not(windows))]
fn move_no_replace(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Moving files is supported on Windows only.",
    ))
}

#[derive(Clone, Debug)]
pub struct MovedFile {
    pub source: PathBuf,
    pub destination: PathBuf,
    digest: [u8; 32],
}

#[derive(Default)]
pub struct BatchResult {
    pub moved: Vec<MovedFile>,
    pub errors: Vec<String>,
}

/// A failed item stops the batch. Earlier successful moves remain undoable.
pub fn execute(items: &[PlannedMove]) -> BatchResult {
    let mut result = BatchResult::default();
    for item in items {
        let op = || -> io::Result<MovedFile> {
            let digest = fingerprint(&item.source)?;
            let parent = item
                .destination
                .parent()
                .ok_or_else(|| io::Error::other("Invalid destination."))?;
            fs::create_dir_all(parent)?;
            // Revalidate redirects just before moving, including changed category folders.
            if fs::canonicalize(parent)? != parent {
                return Err(io::Error::other(
                    "Destination changed since preview. Refresh the preview.",
                ));
            }
            move_no_replace(&item.source, &item.destination)?;
            Ok(MovedFile {
                source: item.source.clone(),
                destination: item.destination.clone(),
                digest,
            })
        };
        match op() {
            Ok(moved) => result.moved.push(moved),
            Err(e) => {
                result
                    .errors
                    .push(format!("{}: {e}", item.source.display()));
                break;
            }
        }
    }
    result
}

/// Undo refuses modified files and occupied original paths; failed items are retained for retry.
pub fn undo(history: &mut Vec<MovedFile>) -> Vec<String> {
    let mut errors = Vec::new();
    let mut remaining = Vec::new();
    for item in history.drain(..).rev() {
        let op = || -> io::Result<()> {
            let parent = item
                .source
                .parent()
                .ok_or_else(|| io::Error::other("Invalid original folder."))?;
            if fs::canonicalize(parent)? != parent {
                return Err(io::Error::other(
                    "The original folder was redirected after sorting; undo skipped.",
                ));
            }
            if fingerprint(&item.destination)? != item.digest {
                return Err(io::Error::other(
                    "File changed after sorting; undo skipped to protect your edits.",
                ));
            }
            move_no_replace(&item.destination, &item.source)
        };
        if let Err(e) = op() {
            errors.push(format!("{}: {e}", item.destination.display()));
            remaining.push(item);
        }
    }
    remaining.reverse();
    *history = remaining;
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn locked_source_remains_untouched() {
        use std::os::windows::fs::OpenOptionsExt;
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("locked.txt");
        fs::write(&p, "keep me").unwrap();
        let items = plan(std::slice::from_ref(&p), t.path()).unwrap();
        let _lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&p)
            .unwrap();
        let result = execute(&items);
        assert_eq!(result.errors.len(), 1);
        assert!(result.moved.is_empty());
        assert!(p.exists());
        assert!(!items[0].destination.exists());
    }
    #[cfg(windows)]
    #[test]
    fn empty_file_can_be_restored_after_missing_parent_is_recreated() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("inbox");
        fs::create_dir(&dir).unwrap();
        let p = dir.join("empty.txt");
        fs::write(&p, []).unwrap();
        let mut result = execute(&plan(std::slice::from_ref(&p), t.path()).unwrap());
        assert!(result.errors.is_empty());
        fs::remove_dir(&dir).unwrap();
        assert_eq!(undo(&mut result.moved).len(), 1);
        assert_eq!(result.moved.len(), 1);
        fs::create_dir(&dir).unwrap();
        assert!(undo(&mut result.moved).is_empty());
        assert_eq!(fs::metadata(p).unwrap().len(), 0);
    }
    #[test]
    fn categories_cover_all_groups() {
        for (name, expected) in [
            ("A.PDF", "Documents"),
            ("a.JPG", "Images"),
            ("a.mp4", "Videos"),
            ("a.flac", "Audio"),
            ("a.tar.gz", "Archives"),
            ("a.exe", "Software"),
            ("README", "Other"),
        ] {
            assert_eq!(category(Path::new(name)), expected);
        }
    }
    #[test]
    fn preview_reserves_names_and_deduplicates() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a");
        let b = t.path().join("b");
        let out = t.path().join("out");
        for p in [&a, &b, &out] {
            fs::create_dir(p).unwrap();
        }
        let x = a.join("report.txt");
        let y = b.join("report.txt");
        fs::write(&x, "one").unwrap();
        fs::write(&y, "two").unwrap();
        let p = plan(&[x.clone(), y, x], &out).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[1].destination.file_name().unwrap(), "report (1).txt");
        assert!(!out.join("Documents").exists());
    }
    #[test]
    fn folders_rejected() {
        let t = tempfile::tempdir().unwrap();
        assert!(plan(&[t.path().to_path_buf()], t.path()).is_err());
    }
    #[test]
    fn already_sorted_is_skipped() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir(t.path().join("Documents")).unwrap();
        let p = t.path().join("Documents/a.txt");
        fs::write(&p, "a").unwrap();
        assert!(plan(&[p], t.path()).unwrap().is_empty());
    }
    #[cfg(windows)]
    #[test]
    fn moves_and_undo_roundtrip_unicode() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("گزارش.txt");
        fs::write(&p, "original").unwrap();
        let mut r = execute(&plan(std::slice::from_ref(&p), t.path()).unwrap());
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert!(!p.exists());
        assert!(undo(&mut r.moved).is_empty());
        assert_eq!(fs::read_to_string(p).unwrap(), "original");
    }
    #[cfg(windows)]
    #[test]
    fn stale_preview_does_not_overwrite() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("a.txt");
        fs::write(&p, "original").unwrap();
        let items = plan(std::slice::from_ref(&p), t.path()).unwrap();
        fs::create_dir(t.path().join("Documents")).unwrap();
        fs::write(&items[0].destination, "new").unwrap();
        let r = execute(&items);
        assert_eq!(r.errors.len(), 1);
        assert_eq!(fs::read_to_string(p).unwrap(), "original");
        assert_eq!(fs::read_to_string(&items[0].destination).unwrap(), "new");
    }
    #[cfg(windows)]
    #[test]
    fn undo_protects_edited_and_recreated_files() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("a.txt");
        fs::write(&p, "original").unwrap();
        let mut r = execute(&plan(std::slice::from_ref(&p), t.path()).unwrap());
        let dest = r.moved[0].destination.clone();
        fs::write(&dest, "edited").unwrap();
        assert_eq!(undo(&mut r.moved).len(), 1);
        assert_eq!(r.moved.len(), 1);
        fs::write(&dest, "original").unwrap();
        fs::write(&p, "replacement").unwrap();
        assert_eq!(undo(&mut r.moved).len(), 1);
        assert_eq!(fs::read_to_string(p).unwrap(), "replacement");
    }
    #[cfg(windows)]
    #[test]
    fn partial_batch_can_be_undone() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a.txt");
        let b = t.path().join("b.txt");
        fs::write(&a, "a").unwrap();
        fs::write(&b, "b").unwrap();
        let items = plan(&[a.clone(), b.clone()], t.path()).unwrap();
        fs::remove_file(b).unwrap();
        let mut r = execute(&items);
        assert_eq!(r.moved.len(), 1);
        assert_eq!(r.errors.len(), 1);
        assert!(undo(&mut r.moved).is_empty());
        assert!(a.exists());
    }
}
