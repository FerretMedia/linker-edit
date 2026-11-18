// Which file GET /library/<folder id>/<path> may send. Remote access (Tailscale) reaches this server, so only a
// playable file that is really inside a library folder goes out: the path is resolved through every link first,
// and anything else is refused.
//
// Sending it is the HTTP layer's: http.ts's sendFile (Range requests, 206/416, `accept-ranges`), with
// `x-content-type-options: nosniff` and `content-security-policy: default-src 'none'; sandbox` on library files.

use std::path::{Component, Path, PathBuf};

use linker_edit_protocol::LibraryFolder;

use crate::js;

/// What the library page plays or shows. Nothing a browser would run as a page or a script: HTML and SVG from a
/// library folder would run on this app's own address, next to its WebSocket.
fn playable(ext: &str) -> Option<&'static str> {
    Some(match ext {
        ".mp3" => "audio/mpeg",
        ".wav" => "audio/wav",
        ".m4a" => "audio/mp4",
        ".aac" => "audio/aac",
        ".ogg" | ".opus" => "audio/ogg",
        ".flac" => "audio/flac",
        ".aif" | ".aiff" => "audio/aiff",
        ".mp4" | ".m4v" => "video/mp4",
        ".mov" => "video/quicktime",
        ".webm" => "video/webm",
        ".png" => "image/png",
        ".jpg" | ".jpeg" => "image/jpeg",
        ".gif" => "image/gif",
        ".webp" => "image/webp",
        _ => return None,
    })
}

/// Is target inside root (not root itself)? Both already resolved; Windows compares without case.
pub fn is_inside(root: impl AsRef<Path>, target: impl AsRef<Path>) -> bool {
    let key = |component: Component| {
        let text = component.as_os_str().to_string_lossy().into_owned();
        if cfg!(windows) {
            text.to_lowercase()
        } else {
            text
        }
    };
    let root: Vec<_> = root.as_ref().components().map(key).collect();
    let target: Vec<_> = target.as_ref().components().map(key).collect();
    target.len() > root.len() && target.starts_with(&root)
}

/// A library file the HTTP layer may send, and its content type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedFile {
    pub path: PathBuf,
    pub content_type: &'static str,
}

/// fs.realpathSync.native: links followed, and on Windows without the \\?\ canonicalize puts in front.
fn real_path(path: &Path) -> std::io::Result<PathBuf> {
    let real = std::fs::canonicalize(path)?;
    let text = real.to_string_lossy();
    Ok(if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{unc}"))
    } else if let Some(local) = text.strip_prefix(r"\\?\") {
        PathBuf::from(local)
    } else {
        real
    })
}

/// The file to send for a folder id and a path inside that folder (/ between folders), or None. Refuses an unknown
/// folder, an absolute path or drive, "..", "." and hidden names, a file a link leads out of the folder, a missing
/// file, a folder, and any file the page doesn't play.
pub fn library_file(folders: &[LibraryFolder], folder_id: &str, rel: &str) -> Option<ServedFile> {
    let folder = folders.iter().find(|candidate| candidate.id == folder_id)?;
    if rel.is_empty() || rel.contains('\0') {
        return None;
    }
    let parts: Vec<&str> = rel.split(['\\', '/']).collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || part.starts_with('.') || part.contains(':'))
    {
        return None;
    }
    let content_type = playable(&js::extname(rel).to_lowercase())?;
    let root = real_path(Path::new(&folder.path)).ok()?;
    let real = real_path(
        &parts
            .iter()
            .fold(root.clone(), |path, part| path.join(part)),
    )
    .ok()?;
    if !is_inside(&root, &real) || !std::fs::metadata(&real).ok()?.is_file() {
        return None;
    }
    Some(ServedFile {
        path: real,
        content_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::testing::{folder, link, temp, write};

    struct Setup {
        _dir: tempfile::TempDir,
        _outside: tempfile::TempDir,
        dir: PathBuf,
        outside: PathBuf,
        folders: Vec<LibraryFolder>,
        linked_folder: bool,
        linked_file: bool,
    }

    fn setup() -> Setup {
        let dir = temp();
        let outside = temp();
        let (d, o) = (dir.path().to_path_buf(), outside.path().to_path_buf());
        write(&d.join("Song.mp3"), "x");
        write(&d.join("Sub").join("Logo.png"), "x");
        write(&d.join("page.html"), "<script>alert(1)</script>");
        write(&d.join("logo.svg"), "<svg onload='alert(1)'/>");
        write(&d.join(".hidden.mp3"), "x");
        write(&o.join("Secret.mp3"), "x");
        let linked_folder = link(&o, &d.join("Out"), true);
        let linked_file = link(&o.join("Secret.mp3"), &d.join("Secret.mp3"), false);
        let folders = vec![folder(&d.to_string_lossy(), "music", "lib")];
        Setup {
            _dir: dir,
            _outside: outside,
            dir: d,
            outside: o,
            folders,
            linked_folder,
            linked_file,
        }
    }

    impl Setup {
        fn serves(&self, rel: &str) -> Option<PathBuf> {
            library_file(&self.folders, "lib", rel).map(|file| file.path)
        }
    }

    #[test]
    fn serves_playable_files_inside_the_folder() {
        let s = setup();
        assert_eq!(
            s.serves("Song.mp3"),
            Some(real_path(&s.dir.join("Song.mp3")).unwrap())
        );
        assert_eq!(
            library_file(&s.folders, "lib", "Sub/Logo.png").map(|file| file.content_type),
            Some("image/png")
        );
    }

    #[test]
    fn refuses_what_isnt_a_playable_file_inside_the_folder() {
        let s = setup();
        let out = s
            .outside
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let secret = s.outside.join("Secret.mp3").to_string_lossy().into_owned();
        for rel in [
            format!("../{out}/Secret.mp3"),
            format!("Sub/../../{out}/Secret.mp3"),
            format!("Sub\\..\\..\\{out}\\Secret.mp3"),
            "./Song.mp3".into(),
            secret,
            "/etc/hosts.mp3".into(),
            "C:/Windows/Media/tada.wav".into(),
            "C:Song.mp3".into(),
            "D:\\Music\\Song.mp3".into(),
            "\\\\server\\share\\Song.mp3".into(),
            ".hidden.mp3".into(),
            "Missing.mp3".into(),
            "Sub".into(),
            "page.html".into(),
            "logo.svg".into(),
            "Song.mp3:stream".into(),
            String::new(),
        ] {
            assert_eq!(s.serves(&rel), None, "{rel}");
        }
    }

    #[test]
    fn refuses_a_folder_it_doesnt_know() {
        let s = setup();
        assert_eq!(library_file(&s.folders, "other", "Song.mp3"), None);
    }

    #[test]
    fn refuses_files_that_links_lead_out_of_the_folder_to() {
        let s = setup();
        assert!(s.linked_folder);
        assert_eq!(s.serves("Out/Secret.mp3"), None);
        if s.linked_file {
            assert_eq!(s.serves("Secret.mp3"), None);
        }
    }
}
