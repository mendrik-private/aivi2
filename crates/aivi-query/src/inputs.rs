use std::{path::PathBuf, sync::Arc};

use crate::RootDatabase;

/// Stable handle for one source file input stored in the query database.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceFile {
    pub(crate) id: u32,
}

impl SourceFile {
    /// Create or reopen a source file input in the database.
    pub fn new(db: &RootDatabase, path: PathBuf, text: String) -> Self {
        db.open_file(path, text)
    }

    /// Replace the text for this source input. Returns true when the revision changed.
    pub fn set_text(self, db: &RootDatabase, text: String) -> bool {
        db.set_text(self, text)
    }

    /// Return the current text snapshot for this file.
    pub fn text(self, db: &RootDatabase) -> String {
        db.source_input(self).source.text().to_owned()
    }

    /// Return the current path snapshot for this file.
    pub fn path(self, db: &RootDatabase) -> PathBuf {
        db.source_input(self).source.path().to_path_buf()
    }

    /// Return the current source snapshot for this file.
    pub fn source(self, db: &RootDatabase) -> Arc<aivi_base::SourceFile> {
        db.source_input(self).source
    }

    /// Return the current revision for this file.
    pub fn revision(self, db: &RootDatabase) -> u64 {
        db.source_input(self).revision
    }
}
