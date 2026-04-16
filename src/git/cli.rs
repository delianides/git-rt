//! Shell-out to the `git` CLI for fast status enumeration.
//!
//! On large repos (300k+ files), `gix::status().into_index_worktree_iter()`
//! takes minutes. `git status --porcelain=v2` does the same job in seconds
//! because git has 20 years of optimization (untracked cache, fsmonitor,
//! parallel walk). This module wraps the CLI call + numstat output and
//! merges them into the existing `FileEntry` shape.

#[allow(unused_imports)]
use std::path::Path;
#[allow(unused_imports)]
use std::process::Command;

#[allow(unused_imports)]
use crate::git::{FileEntry, FileStatus, GitFailure};
