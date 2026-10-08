use crate::qq::Directive;
use anyhow::{Result, ensure};
use std::{
    fs,
    io,
    path::{Path, PathBuf},
};

fn checked_user_file(user_root: &Path, path: &Path, max_bytes: u64) -> Result<PathBuf> {
    let root = fs::canonicalize(user_root)?;
    let file = fs::canonicalize(path)?;
    ensure!(file.starts_with(root), "attachment is outside this user's directories");
    let metadata = fs::metadata(&file)?;
    ensure!(metadata.is_file(), "attachment is not a regular file");
    ensure!(
        metadata.len() <= max_bytes,
        "attachment exceeds the configured size limit"
    );
    Ok(file)
}

pub(super) fn validate_directive(
    user_root: &Path,
    directive: &Directive,
    max_bytes: u64,
) -> Result<()> {
    let path = match directive {
        Directive::Image { path } | Directive::File { path, .. } => path,
    };
    checked_user_file(user_root, path, max_bytes)?;
    Ok(())
}

/// Normalise agent output BEFORE it enters the durable outbox. Built-in image
/// generation writes under CODEX_HOME; only a generated image from the
/// CURRENT user's CURRENT thread may be copied into that user's workspace.
pub(super) fn prepare_directives(
    user_root: &Path,
    workspace: &Path,
    codex_home: &Path,
    thread: Option<&str>,
    directives: Vec<Directive>,
    max_bytes: u64,
) -> Result<Vec<Directive>> {
    let root = fs::canonicalize(user_root)?;
    let mut resolved = Vec::with_capacity(directives.len());
    for directive in directives {
        let (path, is_image) = match &directive {
            Directive::Image { path } => (path, true),
            Directive::File { path, .. } => (path, false),
        };
        let source = fs::canonicalize(path)?;
        let metadata = fs::metadata(&source)?;
        ensure!(metadata.is_file(), "attachment is not a regular file");
        ensure!(metadata.len() <= max_bytes, "attachment exceeds the configured size limit");
        let local = if source.starts_with(&root) {
            source
        } else {
            ensure!(is_image, "attachment is outside this user's directories");
            let thread = thread.ok_or_else(|| anyhow::anyhow!("no active Codex thread for image"))?;
            ensure!(
                !thread.is_empty() && thread.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
                "invalid Codex thread ID"
            );
            let generated = fs::canonicalize(codex_home.join("generated_images").join(thread))?;
            ensure!(
                source.starts_with(&generated) && source != generated,
                "generated image does not belong to current Codex thread"
            );
            let dst_dir = workspace.join("generated");
            fs::create_dir_all(&dst_dir)?;
            let dst_dir = fs::canonicalize(&dst_dir)?;
            ensure!(dst_dir.starts_with(&root), "user workspace escapes user root");
            let extension = source.extension().and_then(|e| e.to_str()).unwrap_or("png");
            ensure!(
                matches!(extension.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif"),
                "unsupported generated image extension"
            );
            let name = format!("{:x}.{extension}", md5::compute(source.to_string_lossy().as_bytes()));
            let dest = dst_dir.join(name);
            if !dest.exists() {
                let mut input = fs::File::open(&source)?;
                let mut output = fs::OpenOptions::new().write(true).create_new(true).open(&dest)?;
                if let Err(err) = io::copy(&mut input, &mut output) {
                    let _ = fs::remove_file(&dest);
                    return Err(err.into());
                }
                output.sync_all()?;
            }
            checked_user_file(&root, &dest, max_bytes)?
        };
        resolved.push(match directive {
            Directive::Image { .. } => Directive::Image { path: local },
            Directive::File { name, .. } => Directive::File { path: local, name },
        });
    }
    Ok(resolved)
}

