use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub fn write(path: &Path, contents: &[u8]) -> Result<(), String> {
    write_temp_then_rename(path, |file| file.write_all(contents))
}

pub fn copy(source: &Path, dest: &Path) -> Result<(), String> {
    write_temp_then_rename(dest, |dest_file| {
        let mut source_file = File::open(source)?;
        io::copy(&mut source_file, dest_file)?;
        Ok(())
    })
    .map_err(|e| format!("copy {} -> {}: {e}", source.display(), dest.display()))
}

fn write_temp_then_rename<F>(dest: &Path, write_temp: F) -> Result<(), String>
where
    F: FnOnce(&mut File) -> io::Result<()>,
{
    let parent = parent_dir(dest)?;
    fs::create_dir_all(&parent).map_err(|e| format!("create {}: {e}", parent.display()))?;

    let (temp_path, mut temp_file) = create_temp_file(dest, &parent)?;
    let write_result = write_temp(&mut temp_file).and_then(|_| temp_file.sync_all());
    drop(temp_file);

    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("write {}: {error}", temp_path.display()));
    }

    if let Err(error) = fs::rename(&temp_path, dest) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "rename {} -> {}: {error}",
            temp_path.display(),
            dest.display()
        ));
    }

    sync_parent_best_effort(&parent);
    Ok(())
}

fn parent_dir(path: &Path) -> Result<PathBuf, String> {
    if path.file_name().is_none() {
        return Err(format!("path has no file name: {}", path.display()));
    }

    Ok(path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf())
}

fn create_temp_file(dest: &Path, parent: &Path) -> Result<(PathBuf, File), String> {
    for attempt in 0..100 {
        let temp_path = temp_path(dest, parent, attempt)?;
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("create {}: {error}", temp_path.display())),
        }
    }

    Err(format!(
        "could not create temporary file for {} after 100 attempts",
        dest.display()
    ))
}

fn temp_path(dest: &Path, parent: &Path, attempt: u32) -> Result<PathBuf, String> {
    let file_name = dest
        .file_name()
        .ok_or_else(|| format!("path has no file name: {}", dest.display()))?
        .to_string_lossy();
    Ok(parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        attempt
    )))
}

fn sync_parent_best_effort(parent: &Path) {
    if let Ok(parent_file) = File::open(parent) {
        let _ = parent_file.sync_all();
    }
}
