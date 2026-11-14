//! Picture files that image fields name by a path relative to a folder: Odoo's filestore
//! (`ir_attachment.store_fname`, such as `3f/3f5a…`), or a folder of photos next to a CSV file.
//! Only files inside the folder are read (no absolute path, no `..`, no link leading out of it),
//! each at most `MAX_IMAGE_BYTES`, and at most `MAX_FOLDER_BYTES` in one render.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use reportcraft_core::image::Picture;
use reportcraft_core::limits::MAX_IMAGE_BYTES;
use reportcraft_layout::PictureFiles;

/// Most bytes of picture files one render reads (distinct files; repeats come from the cache).
pub const MAX_FOLDER_BYTES: usize = 512 << 20;

/// Most distinct paths remembered in one render.
const MAX_CACHED: usize = 100_000;

/// The pictures of one folder, for one render.
pub struct FolderPictures {
    /// The folder, with links resolved.
    root: PathBuf,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    seen: HashMap<String, Option<Picture>>,
    bytes: usize,
}

impl FolderPictures {
    /// The pictures of `folder`; an error when it isn't a folder that can be read.
    pub fn new(folder: &Path) -> reportcraft_core::Result<Self> {
        let root = folder
            .canonicalize()
            .ok()
            .filter(|p| p.is_dir())
            .ok_or_else(|| reportcraft_core::Error::NotFound(format!("picture folder `{}`", folder.display())))?;
        Ok(Self { root, state: Mutex::new(State::default()) })
    }

    /// The file `path` names inside the folder, or `None` when it would lead out of it.
    fn file(&self, path: &str) -> Option<PathBuf> {
        let relative = Path::new(path);
        if path.is_empty() || path.len() > 1_024 || path.contains('\0') || !relative.components().all(|c| matches!(c, Component::Normal(_))) {
            return None;
        }
        // Resolved, so a link inside the folder can't point outside it.
        let file = self.root.join(relative).canonicalize().ok()?;
        file.starts_with(&self.root).then_some(file)
    }

    fn load(&self, path: &str, budget: usize) -> Option<Picture> {
        let file = self.file(path)?;
        let meta = std::fs::metadata(&file).ok()?;
        let len = usize::try_from(meta.len()).ok()?;
        if !meta.is_file() || len > MAX_IMAGE_BYTES || len > budget {
            return None;
        }
        Picture::read(&std::fs::read(file).ok()?).ok()
    }
}

impl PictureFiles for FolderPictures {
    fn read(&self, path: &str) -> Option<Picture> {
        let Ok(mut state) = self.state.lock() else { return None };
        if let Some(p) = state.seen.get(path) {
            return p.clone();
        }
        let picture = self.load(path, MAX_FOLDER_BYTES.saturating_sub(state.bytes));
        state.bytes = state.bytes.saturating_add(picture.as_ref().map_or(0, |p| p.data.0.len()));
        if state.seen.len() < MAX_CACHED {
            state.seen.insert(path.to_owned(), picture.clone());
        }
        picture
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        reportcraft_layout::pixels::encode_png(&reportcraft_layout::pixels::Rgba { width: 2, height: 1, pixels: vec![255; 8] }).unwrap()
    }

    #[test]
    fn reads_pictures_inside_the_folder_only() {
        let base = std::env::temp_dir().join(format!("reportcraft-picture-folder-{}", std::process::id()));
        let root = base.join("filestore");
        std::fs::create_dir_all(root.join("3f")).unwrap();
        std::fs::write(root.join("3f/3f5a01"), png()).unwrap();
        std::fs::write(root.join("notes.txt"), b"not a picture").unwrap();
        std::fs::write(base.join("outside.png"), png()).unwrap();
        let files = FolderPictures::new(&root).unwrap();
        let p = files.read("3f/3f5a01").unwrap();
        assert_eq!((p.width_px, p.height_px), (2, 1));
        // The second read comes from the cache: the same bytes.
        assert!(std::sync::Arc::ptr_eq(&files.read("3f/3f5a01").unwrap().data.0, &p.data.0));
        for refused in ["../outside.png", "3f/../../outside.png", "/etc/passwd", "", "notes.txt", "missing", "3f"] {
            assert!(files.read(refused).is_none(), "{refused}");
        }
        let absolute = base.join("outside.png");
        assert!(files.read(absolute.to_str().unwrap()).is_none());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(base.join("outside.png"), root.join("link.png")).unwrap();
            assert!(files.read("link.png").is_none(), "a link leading out of the folder");
        }
        assert!(FolderPictures::new(&base.join("nope")).is_err());
        assert!(FolderPictures::new(&root.join("notes.txt")).is_err());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
