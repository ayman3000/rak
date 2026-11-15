//! Writing files that hold keys: the key file, its backups, the MCP server
//! list. The file is readable by its owner only from the moment it exists
//! (made with mode 0600, not changed to it after the text is in), and it is
//! written beside the target and renamed over it, so a failed write never
//! leaves half a file.

use std::io::Write;
use std::path::Path;

/// Writes `text` to `file`, readable by its owner only.
pub fn write(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(folder) = file.parent().filter(|f| !f.as_os_str().is_empty()) {
        std::fs::create_dir_all(folder)?;
    }
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(".new");
    let temporary = file.with_file_name(name);
    // A leftover from a failed write may have looser permissions: start fresh.
    let _ = std::fs::remove_file(&temporary);
    let result = create_owner_only(&temporary).and_then(|mut out| {
        out.write_all(text.as_bytes())?;
        out.sync_all()
    });
    match result.and_then(|()| std::fs::rename(&temporary, file)) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&temporary);
            Err(e)
        }
    }
}

/// A new file (never an existing one), owner-only on Unix from creation.
fn create_owner_only(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::temp_dir;

    #[test]
    fn a_key_file_is_owner_only_and_replaced_whole() {
        let dir = temp_dir("private-file");
        let file = dir.join("sub/.env");
        write(&file, "KEY=one\n").unwrap();
        // A stale temporary file with loose permissions is not reused.
        std::fs::write(dir.join("sub/.env.new"), "old").unwrap();
        write(&file, "KEY=two\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "KEY=two\n");
        assert!(!dir.join("sub/.env.new").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
